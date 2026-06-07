//! Per-invariant evaluator functions for the Phase 1 default catalogue.
//!
//! Each function takes the minimum state it needs to decide whether its
//! invariant holds, and returns an [`InvariantResult`]. The harness
//! ([`super::super::harness::Harness`]) composes these evaluators over
//! the live [`Host`]s it owns; individual evaluators are also unit-
//! testable without booting the full harness — see
//! `crates/overdrive-sim/tests/invariant_evaluators.rs`.
//!
//! # Why evaluators are per-function rather than per-trait
//!
//! A single `trait InvariantEvaluator` would have to carry the union of
//! every evaluator's inputs (an intent store, an observation cluster,
//! an entropy seed, ...). Free functions keep each evaluator's surface
//! narrow to the state it actually reads. The harness dispatches on the
//! [`super::Invariant`] enum and calls the matching function directly —
//! no dynamic dispatch, no erased interface, every caller's input list
//! checked at compile time.
//!
//! # Phase 1 scope
//!
//! The `SingleLeader` evaluator operates against a stubbed leader
//! topology per US-06 Technical Note 3: a simple state machine in the
//! harness designates one host as the leader for each epoch. The
//! Phase 2 step that adds `RaftStore` retires this stub and replaces it
//! with a read against the real Raft leader term. The stub is documented
//! inline in the evaluator body.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use overdrive_core::UnixInstant;
use overdrive_core::id::{ContentHash, CorrelationKey, NodeId, WorkloadId};
use overdrive_core::reconcilers::Action;
use overdrive_core::reconcilers::{
    AnyReconciler, AnyReconcilerView, AnyState, NoopHeartbeat, Reconciler, TickContext,
    WorkloadLifecycle,
};
use overdrive_core::testing::workflow::{
    ProvisionRecord, ProvisionRecordWithSignalEmit, ProvisionRecordWithSleep,
};
use overdrive_core::traits::clock::Clock;
use overdrive_core::traits::entropy::Entropy;
use overdrive_core::traits::intent_store::IntentStore;
use overdrive_core::traits::observation_store::{
    AllocState, AllocStatusRow, ObservationRow, ObservationStore,
};
use overdrive_core::traits::transport::Transport as TransportTrait;
use overdrive_core::workflow::{
    JournalCursor, RetryableStepError, SignalValue, TerminalError, TerminalErrorKind, Workflow,
    WorkflowCtx, WorkflowName, WorkflowStart, WorkflowStatus,
};

use overdrive_control_plane::journal::{
    JournalCommand, JournalNotification, JournalStore, LoadedEntry, WorkflowId,
};
use overdrive_control_plane::workflow_runtime::{
    JournalCursorHandle, WORKFLOW_RETRY_BUDGET, WorkflowEngine, WorkflowRegistry,
};

use crate::adapters::clock::SimClock;
use crate::adapters::entropy::SimEntropy;
use crate::adapters::journal::SimJournalStore;
use crate::adapters::observation_store::{
    SimObservationCluster, SimObservationStore, check_lww_convergence,
};
use crate::adapters::transport::{SimInbox, SimTransport};
use crate::harness::{InvariantResult, InvariantStatus};

/// Default reporting tick emitted by Phase 1 evaluators. Later phases
/// replace this with the violating tick on failure.
const REPORT_TICK: u64 = 1_000;

/// Default reporting host used when an evaluator produces a
/// cluster-wide verdict (no single host is responsible).
const CLUSTER_HOST: &str = "cluster";

/// Invariant name — owner of the kebab-case mapping lives in
/// [`super::Invariant`]. Helper so every evaluator returns a result
/// pinned to the canonical string.
fn result(
    name: &str,
    status: InvariantStatus,
    host: &str,
    cause: Option<String>,
) -> InvariantResult {
    InvariantResult {
        name: name.to_owned(),
        status,
        tick: REPORT_TICK,
        host: host.to_owned(),
        cause,
    }
}

// ---------------------------------------------------------------------------
// SingleLeader
// ---------------------------------------------------------------------------

/// Evaluate `SingleLeader` against a stubbed 3-host topology.
///
/// The stub: `hosts` is the full participant list; `leader` is the one
/// host the stub has elected for the current epoch (or `None` if the
/// stub has not yet converged). The invariant holds iff exactly one
/// host is a leader, and that host is in the participant list.
///
/// Phase 2 replaces this with a read against the real Raft leader
/// term — the stub is exercised only by the in-harness `SingleLeader`
/// evaluation until then (US-06 Technical Note 3).
#[must_use]
pub fn evaluate_single_leader_from_topology(
    hosts: &[NodeId],
    leader: Option<&NodeId>,
) -> InvariantResult {
    let name = "single-leader";
    match leader {
        None => result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some("no host claims leader — stub failed to converge".to_owned()),
        ),
        Some(l) if hosts.contains(l) => result(name, InvariantStatus::Pass, &l.to_string(), None),
        Some(l) => result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some(format!("leader {l} is not in the participant set")),
        ),
    }
}

/// Evaluate `SingleLeader` against an explicit leader set — the shape the
/// Phase 1 unit test uses to plant a "two hosts claim leader" failure.
///
/// Exactly one entry passes; zero or more-than-one fails.
#[must_use]
pub fn evaluate_single_leader_from_leader_set(leaders: &[NodeId]) -> InvariantResult {
    let name = "single-leader";
    match leaders.len() {
        1 => result(name, InvariantStatus::Pass, &leaders[0].to_string(), None),
        0 => result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some("zero hosts claim leader".to_owned()),
        ),
        n => result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some(format!("{n} hosts claim leader simultaneously")),
        ),
    }
}

// ---------------------------------------------------------------------------
// IntentNeverCrossesIntoObservation
// ---------------------------------------------------------------------------

/// Observation-class key prefixes. A key in the `IntentStore` whose
/// bytes start with any of these is a §4 guardrail violation — the
/// write went into the wrong store.
const OBSERVATION_KEY_PREFIXES: &[&[u8]] =
    &[b"alloc_status/", b"node_health/", b"service_backends/"];

/// Evaluate `IntentNeverCrossesIntoObservation` for a single-host pair.
///
/// Inspects the intent store for any observation-class key prefix.
/// The observation side of the check is structural, not runtime — the
/// type system closes the other direction (see
/// `crates/overdrive-core/src/traits/observation_store.rs` for the
/// compile-fail shape that rejects intent-class rows on `write`). For
/// the runtime invariant we scan the intent keyspace and report any
/// observation-prefix match.
pub async fn evaluate_intent_crossing(
    intent: &impl IntentStore,
    _observation: &SimObservationStore,
) -> InvariantResult {
    let name = "intent-never-crosses-into-observation";
    // Snapshot once, then scan every banned prefix against the same
    // view. Scanning inside the loop would pay for N redb roundtrips
    // and — worse — race against concurrent writers under Phase 2
    // multi-writer scenarios, meaning a prefix that only showed up
    // after the first snapshot could slip past the second scan and
    // back before the third. A single atomic snapshot closes the
    // window.
    let snap = match intent.export_snapshot().await {
        Ok(s) => s,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("intent snapshot failed: {err}")),
            );
        }
    };
    for prefix in OBSERVATION_KEY_PREFIXES {
        // `get` on a prefix key directly is not enough — we need to
        // check whether *any* key in intent starts with one of the
        // banned prefixes. `watch` + a short drain would work but is
        // racy for a one-shot probe. Instead, we probe for the exact
        // prefix; a production writer of observation-class data into
        // intent would almost certainly write the prefix verbatim
        // (the failure shape the test exercises) plus some alloc id
        // suffix.
        for (k, _v) in &snap.entries {
            if k.starts_with(prefix) {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!(
                        "intent store holds observation-prefix key: {:?}",
                        String::from_utf8_lossy(k),
                    )),
                );
            }
        }
    }
    result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
}

// ---------------------------------------------------------------------------
// SnapshotRoundtripBitIdentical
// ---------------------------------------------------------------------------

/// Evaluate the snapshot roundtrip invariant against `intent`.
///
/// Drives the step 03-02 logic from within the harness: export,
/// bootstrap a second `LocalIntentStore` from the frame, re-export, and
/// compare bytes.
pub async fn evaluate_snapshot_roundtrip(intent: &impl IntentStore) -> InvariantResult {
    let name = "snapshot-roundtrip-bit-identical";

    let first = match intent.export_snapshot().await {
        Ok(s) => s,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("first export failed: {err}")),
            );
        }
    };

    // Bootstrap a fresh LocalIntentStore from the frame.
    let tmp = match tempfile::tempdir() {
        Ok(t) => t,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("tempdir for roundtrip failed: {err}")),
            );
        }
    };
    let path = tmp.path().join("roundtrip.redb");
    let second_store = match overdrive_store_local::LocalIntentStore::open(&path) {
        Ok(s) => s,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("second open failed: {err}")),
            );
        }
    };
    if let Err(err) = second_store.bootstrap_from(first.clone()).await {
        return result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some(format!("bootstrap_from failed: {err}")),
        );
    }
    let second = match second_store.export_snapshot().await {
        Ok(s) => s,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("second export failed: {err}")),
            );
        }
    };

    if first.bytes() == second.bytes() {
        result(name, InvariantStatus::Pass, "host-0", None)
    } else {
        result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some(format!(
                "roundtrip bytes differ: first={} second={}",
                first.bytes().len(),
                second.bytes().len(),
            )),
        )
    }
}

// ---------------------------------------------------------------------------
// IntentStoreReturnsCallerBytes
// ---------------------------------------------------------------------------

/// Probe keys for [`evaluate_intent_store_returns_caller_bytes`]. Owns
/// each [`overdrive_core::aggregate::IntentKey`] so the slice borrows
/// returned by `as_bytes()` outlive the evaluator's loop.
struct IntentStoreProbeKeys {
    empty: overdrive_core::aggregate::IntentKey,
    le_prefix: overdrive_core::aggregate::IntentKey,
    long: overdrive_core::aggregate::IntentKey,
}

/// Derive the three probe keys via `IntentKey::for_workload` so the
/// literal `workloads/` prefix is sourced from `aggregate/mod.rs`
/// (the SSOT) per ADR-0050 OQ-5 single-cut migration.
fn derive_intent_store_probe_keys() -> Result<IntentStoreProbeKeys, String> {
    let mk = |raw: &str| -> Result<overdrive_core::aggregate::IntentKey, String> {
        overdrive_core::id::WorkloadId::new(raw)
            .map(|id| overdrive_core::aggregate::IntentKey::for_workload(&id))
            .map_err(|e| format!("derive IntentKey for {raw}: {e}"))
    };
    Ok(IntentStoreProbeKeys {
        empty: mk("k-empty")?,
        le_prefix: mk("k-le-prefix")?,
        long: mk("k-long")?,
    })
}

/// Evaluate the structural-regression guard from ADR-0020 §Enforcement.
///
/// Writes a small fixed set of `(key, value)` pairs through
/// [`IntentStore::put`] and [`IntentStore::put_if_absent`], then reads
/// each key back and checks the returned bytes are byte-identical to
/// the bytes that went in. This catches any future re-introduction of
/// inline framing (`[u64-LE-prefix || value]`) or any other on-disk
/// row encoding that would surface a transformed value at the trait
/// boundary.
///
/// The fixtures cover the failure shapes inline framing produces:
/// * an empty value (an inline u64 prefix would surface as 8 bytes)
/// * a value whose bytes start with what would be a plausible u64-LE
///   prefix (an inline framing would slice off the first 8 bytes)
/// * a long value (general round-trip witness)
///
/// The invariant uses a fresh tempdir-backed `LocalIntentStore` rather
/// than the harness's per-host store so it cannot interact with state
/// other invariants leave behind.
pub async fn evaluate_intent_store_returns_caller_bytes() -> InvariantResult {
    let name = "intent-store-returns-caller-bytes";

    let tmp = match tempfile::tempdir() {
        Ok(t) => t,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("tempdir for intent-store probe failed: {err}")),
            );
        }
    };
    let path = tmp.path().join("caller-bytes.redb");
    let store = match overdrive_store_local::LocalIntentStore::open(&path) {
        Ok(s) => s,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("intent store open failed: {err}")),
            );
        }
    };

    // Fixtures chosen to flush inline-framing regressions:
    //   * empty value — inline framing would surface 8 prefix bytes
    //   * 8-byte LE-looking prefix — inline framing would slice it off
    //   * arbitrary 32-byte payload — general round-trip witness
    //
    // Keys are derived through `IntentKey::for_job` so the literal
    // `jobs/` prefix appears in exactly one production file (the
    // SSOT in `aggregate/mod.rs`); the canonical-key grep gate in
    // `overdrive-core/tests/acceptance/intent_key_canonical.rs`
    // enforces this.
    let keys = match derive_intent_store_probe_keys() {
        Ok(k) => k,
        Err(err) => {
            return result(name, InvariantStatus::Fail, "host-0", Some(err));
        }
    };
    let fixtures: &[(&[u8], &[u8])] = &[
        (keys.empty.as_bytes(), b""),
        (keys.le_prefix.as_bytes(), &[0x07, 0, 0, 0, 0, 0, 0, 0, b'a', b'b', b'c']),
        (keys.long.as_bytes(), b"the-rain-in-spain-falls-mainly-on"),
    ];

    // Write the first two via `put`, the third via `put_if_absent` so
    // the invariant covers both insert paths. A regression that
    // re-introduces framing on only one path would still be caught.
    for (k, v) in &fixtures[..2] {
        if let Err(err) = store.put(k, v).await {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("put({:?}) failed: {err}", String::from_utf8_lossy(k))),
            );
        }
    }
    if let Err(err) = store.put_if_absent(fixtures[2].0, fixtures[2].1).await {
        return result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some(format!(
                "put_if_absent({:?}) failed: {err}",
                String::from_utf8_lossy(fixtures[2].0),
            )),
        );
    }

    for (k, expected) in fixtures {
        match store.get(k).await {
            Ok(Some(actual)) => {
                if actual.as_ref() != *expected {
                    return result(
                        name,
                        InvariantStatus::Fail,
                        "host-0",
                        Some(format!(
                            "get({key:?}) returned {actual_len} bytes, expected {expected_len}: \
                             actual={actual:?} expected={expected:?}",
                            key = String::from_utf8_lossy(k),
                            actual_len = actual.len(),
                            expected_len = expected.len(),
                            actual = actual.as_ref(),
                            expected = expected,
                        )),
                    );
                }
            }
            Ok(None) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!("get({:?}) returned None after put", String::from_utf8_lossy(k))),
                );
            }
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!("get({:?}) failed: {err}", String::from_utf8_lossy(k))),
                );
            }
        }
    }

    result(name, InvariantStatus::Pass, "host-0", None)
}

// ---------------------------------------------------------------------------
// SimObservationLwwConverges
// ---------------------------------------------------------------------------

/// Evaluate the LWW-convergence invariant against `cluster`.
///
/// Drives the step 04-03 `check_lww_convergence` helper from within the
/// harness. The invariant holds when every peer that has observed an
/// alloc holds the same row for it as every other peer that has
/// observed it.
pub async fn evaluate_sim_observation_lww(cluster: &SimObservationCluster) -> InvariantResult {
    use std::str::FromStr;

    use overdrive_core::id::{AllocationId, WorkloadId};
    use overdrive_core::traits::observation_store::{
        AllocState, AllocStatusRow, LogicalTimestamp, ObservationRow, ObservationStore,
    };

    let name = "sim-observation-lww-converges";

    // Drive two concurrent writes from different peers to the same
    // allocation. Without this, `check_lww_convergence` asserts
    // trivially on an empty cluster — there is nothing for LWW to
    // resolve. The WS-3 canary bug only manifests when LWW actually
    // has to pick between competing timestamps.
    // `cluster.peers()` is a `HashMap::iter()` under the hood — order
    // varies per-run via Rust's default `RandomState` hasher. Sort
    // explicitly so writer[0] vs writer[1] is pinned and K3 bit-for-bit
    // reproducibility holds across invocations of the same seed.
    let mut peers: Vec<NodeId> = cluster.peers().map(|(id, _)| id.clone()).collect();
    peers.sort();
    if peers.len() >= 2 {
        let alloc_id = match AllocationId::from_str("a1b2c3") {
            Ok(a) => a,
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    CLUSTER_HOST,
                    Some(format!("could not construct alloc id: {err}")),
                );
            }
        };
        let workload_id = match WorkloadId::from_str("payments") {
            Ok(j) => j,
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    CLUSTER_HOST,
                    Some(format!("could not construct job id: {err}")),
                );
            }
        };

        // Two writers, two different logical timestamps, same alloc.
        // Counter 1 < counter 2 so LWW has a definitive winner.
        for (i, writer) in peers.iter().take(2).enumerate() {
            let counter = (i as u64) + 1;
            let state = if i == 0 { AllocState::Pending } else { AllocState::Running };
            let row = AllocStatusRow {
                alloc_id: alloc_id.clone(),
                workload_id: workload_id.clone(),
                node_id: writer.clone(),
                state,
                updated_at: LogicalTimestamp { counter, writer: writer.clone() },
                reason: None,
                detail: None,
                terminal: None,
                stderr_tail: None,
                kind: overdrive_core::aggregate::WorkloadKind::Service,
                listeners: Vec::new(),
                // GAP-1 subsidiary: None on Pending; fixed wall-clock
                // on Running. Value arbitrary for this invariant.
                started_at: match state {
                    AllocState::Pending => None,
                    _ => Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
                },
            };
            let peer = cluster.peer(writer);
            if let Err(err) = peer.write(ObservationRow::AllocStatus(Box::new(row))).await {
                return result(
                    name,
                    InvariantStatus::Fail,
                    &writer.to_string(),
                    Some(format!("peer write failed: {err}")),
                );
            }
        }
    }

    // Drain the gossip window after the writes so every peer has seen
    // every row. Two advances past the gossip-delay ceiling so FIFOs
    // fully drain even under the cluster's default delay.
    cluster.advance(Duration::from_millis(500)).await;
    cluster.advance(Duration::from_millis(500)).await;

    let report = check_lww_convergence(cluster);
    if report.is_converged() {
        result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
    } else {
        // Report the first peer with a disagreement so the WS-3 failure
        // block names a concrete host. The report is deterministic
        // under the BTreeMap ordering in ConvergenceReport so "first"
        // is stable across runs.
        let host = report
            .peer_views()
            .keys()
            .next()
            .map_or_else(|| CLUSTER_HOST.to_owned(), ToString::to_string);
        result(
            name,
            InvariantStatus::Fail,
            &host,
            Some("peers disagree on an alloc_status row after gossip drain".to_owned()),
        )
    }
}

// ---------------------------------------------------------------------------
// ReplayEquivalenceProvisionRecord (workflow-primitive step 01-07, K4)
// ---------------------------------------------------------------------------
//
// Graduates the slice-1 `ReplayEquivalentEmptyWorkflow` placeholder
// (two-SimEntropy-transcripts) into a real journal replay driving the
// `WorkflowEngine` + `SimJournalStore` against the `ProvisionRecord`
// reference workflow. ADR-0064 §3/§6. K4 — the load-bearing KPI on the
// `cargo dst` critical path.

/// The fixed address the `ProvisionRecord` reference workflow's single
/// `ctx.run` provision-write effect is addressed at, in every evaluator
/// below.
const WF_TARGET: &str = "127.0.0.1:9000";

/// Construct the slice-01 instance correlation + id + spec for a
/// `ProvisionRecord` instance. Deterministic — no seed dependence beyond
/// the fixed instance id, so twin runs reproduce bit-for-bit.
fn provision_instance() -> (CorrelationKey, WorkflowId, WorkflowStart) {
    let spec = ProvisionRecord::spec();
    let correlation = CorrelationKey::derive(
        "wf-provision-0001",
        &ContentHash::of(ProvisionRecord::WORKFLOW_NAME.as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-provision-0001")
        .unwrap_or_else(|_| unreachable!("wf-provision-0001 is a valid instance id"));
    (correlation, workflow_id, spec)
}

/// The command-index-0 `Started` entry the real `WorkflowEngine::start`
/// uninterrupted path now writes for `spec` (CA-4, ADR-0063 §2 / ADR-0064
/// §5). The crash-run constructors below drive a raw `ctx` via
/// [`JournalCursorHandle::new`] — bypassing `engine.start`, so they never
/// see the engine's `Started` write. Their hand-built crash journals must
/// nonetheless BEGIN with the SAME `Started` the engine writes, or the
/// replay-equivalence oracle (`entry_kinds`) sees the resumed trajectory
/// `[Started, …]` (the resume goes through `engine.start`, which finds the
/// `Started` already present and appends none) diverge from a crash journal
/// that opened with a bare `RunResult`.
///
/// The digests mirror the engine's `started_digests` derivation verbatim —
/// `ContentHash::of(spec.name…)` for both — so the entry is byte-identical
/// to the one `engine.start` would have written. Production is the source
/// of truth (`development.md` § "Production code is not shaped by
/// simulation"); this harness fixture mirrors it.
fn started_entry(spec: &WorkflowStart) -> LoadedEntry {
    let digest = ContentHash::of(spec.name.as_str().as_bytes());
    LoadedEntry::Command(JournalCommand::Started { spec_digest: digest, input_digest: digest })
}

/// Build a `WorkflowEngine` over the SHARED `journal` + `obs`, a fresh set
/// of `Sim*` ports, and a freshly-bound transport inbox so each boot
/// observes its OWN delivered-datagram count. The engine resolves
/// `ProvisionRecord` addressed at [`WF_TARGET`].
async fn provision_engine(
    seed: u64,
    journal: Arc<dyn JournalStore>,
    obs: Arc<dyn ObservationStore>,
) -> (WorkflowEngine, SimInbox) {
    let target: SocketAddr =
        WF_TARGET.parse().unwrap_or_else(|_| unreachable!("WF_TARGET is a valid socket addr"));
    let sim_transport = SimTransport::new();
    let inbox = sim_transport
        .bind_inbox(target)
        .await
        .unwrap_or_else(|_| unreachable!("SimTransport::bind_inbox is total"));

    let transport: Arc<dyn TransportTrait> = Arc::new(sim_transport);
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(seed));

    let mut registry = WorkflowRegistry::new();
    registry.register(ProvisionRecord::spec().name, move || ProvisionRecord::new(target));

    let engine = WorkflowEngine::new(journal, clock, transport, entropy, registry, obs);
    (engine, inbox)
}

/// Count datagrams delivered to `inbox` within the drain budget — the
/// per-boot `SimTransport` effect-fire count.
async fn delivered_count(inbox: &mut SimInbox) -> usize {
    let mut count = 0usize;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), inbox.recv()).await {
        count += 1;
    }
    count
}

/// Drain the terminal `WorkflowStatus` for `correlation` off a
/// subscription that was taken BEFORE the engine drove the workflow to
/// terminal. `SimObservationStore::subscribe_all` returns a LIVE
/// broadcast stream (it does NOT replay a snapshot — `WorkflowTerminal`
/// rows are fan-out-only, never stored), so the subscription MUST be
/// opened before `engine.start` or the terminal row is missed. Returns
/// `None` if no terminal row arrives within the drain budget.
async fn drain_terminal(
    subscription: &mut overdrive_core::traits::observation_store::ObservationSubscription,
    correlation: &CorrelationKey,
) -> Option<WorkflowStatus> {
    use futures::StreamExt;
    for _ in 0..32 {
        match tokio::time::timeout(Duration::from_millis(100), subscription.next()).await {
            Ok(Some(ObservationRow::WorkflowTerminal { correlation: got, status }))
                if &got == correlation =>
            {
                return Some(status);
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => break,
        }
    }
    None
}

/// Drive an uninterrupted `ProvisionRecord` run to terminal through the
/// engine; return the loaded journal trajectory + terminal result.
async fn run_uninterrupted(seed: u64) -> (Vec<LoadedEntry>, Option<WorkflowStatus>) {
    let (correlation, workflow_id, spec) = provision_instance();
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("local").unwrap_or_else(|_| unreachable!("local is a valid node id")),
        0,
    ));
    let (engine, _inbox) = provision_engine(seed, Arc::clone(&journal), Arc::clone(&obs)).await;
    // Subscribe BEFORE driving — the WorkflowTerminal row is broadcast
    // live (never snapshotted), so a post-run subscriber would miss it.
    let Ok(mut subscription) = obs.subscribe_all().await else {
        return (Vec::new(), None);
    };
    let _ = engine.start(&spec, &correlation, &workflow_id).await;
    engine.join_all().await;
    let trajectory = journal.load_journal(&workflow_id).await.unwrap_or_default();
    let terminal = drain_terminal(&mut subscription, &correlation).await;
    (trajectory, terminal)
}

/// Drive a crash-injected `ProvisionRecord` run.
///
/// Runs the `ctx.run("provision-write", ...)` step once (records step 0),
/// then drops the future BEFORE terminal — a process-local kill modelled
/// honestly. Returns the persisted journal (carrying the recorded
/// `RunResult`, NO `Terminal`) and the count of effect fires during this
/// pre-crash run.
async fn run_until_crash(seed: u64, journal: &Arc<dyn JournalStore>) -> (Vec<LoadedEntry>, usize) {
    let (_correlation, workflow_id, spec) = provision_instance();
    let target: SocketAddr =
        WF_TARGET.parse().unwrap_or_else(|_| unreachable!("WF_TARGET is a valid socket addr"));
    let sim_transport = SimTransport::new();
    let mut inbox = sim_transport
        .bind_inbox(target)
        .await
        .unwrap_or_else(|_| unreachable!("bind_inbox is total"));
    // The first start writes `Started` at command-index 0 (CA-4); the real
    // `engine.start` does this on the live path. The crash journal must open
    // with the SAME entry so the resumed trajectory (which re-enters through
    // `engine.start`, sees the `Started` already present, and appends none)
    // is byte-identical to an uninterrupted run. The raw-`ctx` step below
    // then records `RunResult` AFTER it — `[Started, RunResult]`.
    let _ = journal.append(&workflow_id, &started_entry(&spec)).await;
    {
        let cursor: Arc<dyn JournalCursor> = Arc::new(JournalCursorHandle::new(
            Arc::clone(journal),
            workflow_id.clone(),
            Vec::new(),
        ));
        let ctx = WorkflowCtx::new(
            Arc::new(SimClock::new()),
            Arc::new(sim_transport) as Arc<dyn TransportTrait>,
            Arc::new(SimEntropy::new(seed)),
            cursor,
        );
        // The recorded step — the same `ctx.run` durable step
        // `ProvisionRecord` performs. The ctx + future drop at the end of
        // this block model the crash BEFORE terminal.
        let _ = provision_write_step(&ctx, target).await;
    }
    let fires = delivered_count(&mut inbox).await;
    let trajectory = journal.load_journal(&workflow_id).await.unwrap_or_default();
    (trajectory, fires)
}

/// Perform the canonical `provision-write` durable step through `ctx.run`
/// — the same step the `ProvisionRecord` reference workflow runs. Mirrors
/// its body so the evaluators driving a raw ctx exercise the identical
/// recorded-step shape.
async fn provision_write_step(ctx: &WorkflowCtx, target: SocketAddr) -> Result<usize, String> {
    let transport = Arc::clone(ctx.transport());
    let payload = bytes::Bytes::from_static(ProvisionRecord::PAYLOAD);
    ctx.run("provision-write", async move {
        transport.send_datagram(target, payload).await.map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|err| Err(err.to_string()))
}

/// Evaluate `ReplayEquivalenceProvisionRecord` (K4).
///
/// Drives the three-run crash-resume shape (ADR-0064 §3): uninterrupted,
/// crash-injected, resumed-from-journal. Asserts the resumed trajectory is
/// byte-identical to the uninterrupted one AND the resumed run reaches a
/// terminal `WorkflowStatus` (bounded progress).
#[must_use]
pub async fn evaluate_replay_equivalence_provision_record(seed: u64) -> InvariantResult {
    let name = "replay-equivalence-provision-record";
    let fail = |cause: String| result(name, InvariantStatus::Fail, "host-0", Some(cause));

    // (1) Uninterrupted run — the reference trajectory.
    let (uninterrupted, uninterrupted_terminal) = run_uninterrupted(seed).await;
    let Some(uninterrupted_terminal) = uninterrupted_terminal else {
        return fail("uninterrupted run did not reach a terminal WorkflowStatus".to_owned());
    };

    // (2) Crash-injected run on a fresh shared journal — records step 0,
    //     no Terminal.
    let (correlation, workflow_id, spec) = provision_instance();
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let (pre_resume, _crash_fires) = run_until_crash(seed, &journal).await;
    if !pre_resume
        .iter()
        .any(|e| matches!(e, LoadedEntry::Command(JournalCommand::RunResult { .. })))
    {
        return fail(format!("crash run left no recorded RunResult: {pre_resume:?}"));
    }
    if pre_resume.iter().any(|e| matches!(e, LoadedEntry::Command(JournalCommand::Terminal { .. })))
    {
        return fail(format!("crash run wrote a Terminal before the crash: {pre_resume:?}"));
    }

    // (3) Resume from the persisted journal through the engine.
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("local").unwrap_or_else(|_| unreachable!("local is a valid node id")),
        0,
    ));
    let (engine, mut resume_inbox) =
        provision_engine(seed, Arc::clone(&journal), Arc::clone(&obs)).await;
    // Subscribe BEFORE driving the resumed run — the terminal row is
    // broadcast live, not snapshotted.
    let mut resume_sub = match obs.subscribe_all().await {
        Ok(s) => s,
        Err(err) => return fail(format!("resume subscribe_all failed: {err}")),
    };
    let _ = engine.start(&spec, &correlation, &workflow_id).await;
    engine.join_all().await;

    // Exactly-once on resume: the resumed boot re-fired the effect ZERO
    // times (replay short-circuited the recorded call).
    let resume_fires = delivered_count(&mut resume_inbox).await;
    if resume_fires != 0 {
        return fail(format!(
            "resume re-fired the recorded ctx.run effect {resume_fires} times (must be 0)"
        ));
    }

    // assert_eventually!(is_terminal): the resumed run reached terminal.
    let Some(resumed_terminal) = drain_terminal(&mut resume_sub, &correlation).await else {
        return fail(
            "resumed run did not reach a terminal WorkflowStatus (no bounded progress)".to_owned(),
        );
    };

    // Replay-equivalence: the resumed trajectory is byte-identical to the
    // uninterrupted one, and the terminal results match.
    let resumed = journal.load_journal(&workflow_id).await.unwrap_or_default();

    // (b) — Started-at-command-index-0 full-command-sequence equality (D6 /
    // ADR-0064 §6; step 01-06). WIDENS the prior "recorded RunResult matches"
    // equality to a FULL command-kind-sequence equality that pins `Started`
    // at command-index 0 in BOTH runs. This is the regression guard that
    // would have caught the trap: a dropped `Started` write (no `Started` at
    // command-index 0) or a divergent command sequence fails here.
    if let Some(cause) =
        assert_started_at_index_0_and_command_sequence_identical(&resumed, &uninterrupted)
    {
        return fail(cause);
    }
    // The recorded-step contents (each RunResult's name + bytes) match — the
    // command-kind sequence equality above pins the SHAPE; this pins the
    // deterministic CONTENT of the replayed steps.
    if recorded_run_steps(&resumed) != recorded_run_steps(&uninterrupted) {
        return fail(format!(
            "resumed recorded RunResult steps differ from uninterrupted: \
             {resumed:?} vs {uninterrupted:?}"
        ));
    }
    if !resumed.iter().any(|e| matches!(e, LoadedEntry::Command(JournalCommand::Terminal { .. }))) {
        return fail(format!("resumed run did not append a Terminal entry: {resumed:?}"));
    }
    if resumed_terminal != uninterrupted_terminal {
        return fail(format!(
            "resumed terminal {resumed_terminal:?} != uninterrupted {uninterrupted_terminal:?}"
        ));
    }

    // Slice 02 (step 02-02): extend the SAME named invariant — not a new
    // family (the verbatim constraint) — to also exercise the 3-await
    // `ctx.run → ctx.sleep → ctx.run` shape across a crash that SPANS the
    // sleep window. Replay-equivalence (K4) must hold across the durable
    // sleep, seeded and reproducible. SINGLE-NODE only (D3 / #205).
    if let Some(cause) = check_replay_equivalence_across_sleep(seed).await {
        return fail(cause);
    }

    // Slice 03 (step 03-02): extend the SAME named invariant again — not a
    // new family — to also exercise the `ctx.wait_for_signal →
    // ctx.emit_action → terminal` shape across a crash WHILE blocked on the
    // signal. Replay-equivalence (K4) must hold across the durable signal
    // wait + emit, seeded and reproducible. SINGLE-NODE only (D3 / #205).
    if let Some(cause) = check_replay_equivalence_across_signal_emit(seed).await {
        return fail(cause);
    }

    result(name, InvariantStatus::Pass, "host-0", None)
}

// ---------------------------------------------------------------------------
// ReplayEquivalenceProvisionRecord — slice 03 signal+emit extension (03-02)
// ---------------------------------------------------------------------------
//
// Drives `ProvisionRecordWithSignalEmit` (the slice-03 `ctx.wait_for_signal
// → ctx.emit_action → terminal` reference consumer) through a crash-resume
// shape where the crash lands WHILE blocked on the ABSENT signal: the
// pre-crash run records `SignalAwaited` and parks on the absent signal
// (NEVER reaching `SignalSeen` / `ActionEmitted` / `Terminal`); on resume
// the recorded `SignalAwaited` is replayed-past (no duplicate) and the run
// RE-BLOCKS on the SAME signal, then — once the signal is written —
// records `SignalSeen`, emits exactly once, and reaches terminal. The
// resumed trajectory is byte-identical (entry kinds) to an uninterrupted
// run, and the downstream effect fires exactly once across the crash (K1).
// ADR-0063 §2, ADR-0064 §4/§6.

/// The signal value the signal+emit reference workflow's producer writes.
const WF_SIGNAL_VALUE: &str = "provision-signal-ready";

/// Construct the instance correlation + id + spec for a
/// `ProvisionRecordWithSignalEmit` instance. Deterministic — fixed instance
/// id, so twin runs reproduce bit-for-bit.
fn provision_signal_instance() -> (CorrelationKey, WorkflowId, WorkflowStart) {
    let spec = ProvisionRecordWithSignalEmit::spec();
    let correlation = CorrelationKey::derive(
        "wf-provision-signal-inv-0001",
        &ContentHash::of(ProvisionRecordWithSignalEmit::WORKFLOW_NAME.as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-provision-signal-inv-0001")
        .unwrap_or_else(|_| unreachable!("wf-provision-signal-inv-0001 is a valid instance id"));
    (correlation, workflow_id, spec)
}

/// Build a `WorkflowEngine` over the SHARED `journal` + `obs`, a fresh set
/// of `Sim*` ports, resolving `ProvisionRecordWithSignalEmit`. Returns the
/// `SimClock` so the caller can advance logical time (the harness owns
/// logical time — `SimClock::sleep` parks, it never auto-advances).
fn provision_signal_engine(
    seed: u64,
    journal: Arc<dyn JournalStore>,
    obs: Arc<dyn ObservationStore>,
) -> (WorkflowEngine, Arc<SimClock>) {
    let sim_clock = Arc::new(SimClock::new());
    let transport: Arc<dyn TransportTrait> = Arc::new(SimTransport::new());
    let clock: Arc<dyn Clock> = sim_clock.clone();
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(seed));

    let mut registry = WorkflowRegistry::new();
    registry.register(ProvisionRecordWithSignalEmit::spec().name, || {
        ProvisionRecordWithSignalEmit::new(
            ProvisionRecordWithSignalEmit::signal_key(),
            Action::Noop,
        )
    });

    let engine = WorkflowEngine::new(journal, clock, transport, entropy, registry, obs);
    (engine, sim_clock)
}

/// Count Actions emitted on the engine's Action channel within a drain
/// budget — the per-boot emit-fire count.
async fn drained_emit_count(
    rx: &mut overdrive_control_plane::workflow_runtime::ActionEmitReceiver,
) -> usize {
    let mut count = 0usize;
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(20), rx.recv()).await {
        count += 1;
    }
    count
}

/// Write the matching signal row, then drive the engine to terminal,
/// advancing the `SimClock` so the live/replay signal poll resolves.
async fn drive_signal_to_terminal(
    engine: &WorkflowEngine,
    obs: &Arc<dyn ObservationStore>,
    clock: &Arc<SimClock>,
) {
    let _ = obs
        .write(ObservationRow::Signal {
            key: ProvisionRecordWithSignalEmit::signal_key(),
            value: SignalValue::new(WF_SIGNAL_VALUE),
        })
        .await;
    let driver = Arc::clone(clock);
    let ticker = tokio::spawn(async move {
        for _ in 0..16 {
            tokio::task::yield_now().await;
            driver.tick(Duration::from_millis(100));
        }
    });
    engine.join_all().await;
    let _ = ticker.await;
}

/// Drive an uninterrupted `ProvisionRecordWithSignalEmit` run to terminal;
/// return the loaded journal trajectory + terminal result.
async fn run_signal_uninterrupted(seed: u64) -> (Vec<LoadedEntry>, Option<WorkflowStatus>) {
    let (correlation, workflow_id, spec) = provision_signal_instance();
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("local").unwrap_or_else(|_| unreachable!("local is a valid node id")),
        0,
    ));
    let (engine, clock) = provision_signal_engine(seed, Arc::clone(&journal), Arc::clone(&obs));
    let _emits = engine.take_action_emit_receiver().await;
    let Ok(mut subscription) = obs.subscribe_all().await else {
        return (Vec::new(), None);
    };
    let _ = engine.start(&spec, &correlation, &workflow_id).await;
    drive_signal_to_terminal(&engine, &obs, &clock).await;
    let trajectory = journal.load_journal(&workflow_id).await.unwrap_or_default();
    let terminal = drain_terminal(&mut subscription, &correlation).await;
    (trajectory, terminal)
}

/// Drive a `ProvisionRecordWithSignalEmit` run that crashes WHILE blocked
/// on the ABSENT signal: start the run, advance logical time WITHOUT
/// writing the signal (so the wait stays blocked), then drop the engine +
/// task at the end of this scope — a process-local kill DURING the block.
/// Returns `(pre_crash_emit_count, persisted_journal)` on success, or
/// `Err(cause)` if the emit receiver could not be taken.
async fn run_signal_until_crash_while_blocked(
    seed: u64,
    journal: &Arc<dyn JournalStore>,
    obs: &Arc<dyn ObservationStore>,
    spec: &WorkflowStart,
    correlation: &CorrelationKey,
    workflow_id: &WorkflowId,
) -> Result<(usize, Vec<LoadedEntry>), String> {
    let (engine, clock) = provision_signal_engine(seed, Arc::clone(journal), Arc::clone(obs));
    let Some(mut emits) = engine.take_action_emit_receiver().await else {
        return Err("crash engine had no emit receiver".to_owned());
    };
    let _ = engine.start(spec, correlation, workflow_id).await;
    // Advance logical time WITHOUT writing the signal — the wait must stay
    // blocked. The engine + task drop at the end of this fn model the crash
    // WHILE blocked.
    for _ in 0..8 {
        tokio::task::yield_now().await;
        clock.tick(Duration::from_millis(100));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let emit_count = drained_emit_count(&mut emits).await;
    let trajectory = journal.load_journal(workflow_id).await.unwrap_or_default();
    Ok((emit_count, trajectory))
}

/// Slice-03 signal+emit extension of `ReplayEquivalenceProvisionRecord`
/// (K4). Drives a crash WHILE blocked on the absent signal, then resumes
/// and satisfies the signal. Returns `None` on success, or `Some(cause)`
/// naming the first violated property.
async fn check_replay_equivalence_across_signal_emit(seed: u64) -> Option<String> {
    // (1) Uninterrupted run — the reference trajectory across signal+emit.
    let (uninterrupted, uninterrupted_terminal) = run_signal_uninterrupted(seed).await;
    let uninterrupted_terminal = uninterrupted_terminal?;
    for kind in ["SignalAwaited", "SignalSeen", "ActionEmitted"] {
        if !entry_kinds(&uninterrupted).contains(&kind) {
            return Some(format!(
                "uninterrupted signal+emit run missing {kind}: {uninterrupted:?}"
            ));
        }
    }

    // (2) Crash-injected run on a fresh SHARED journal+obs: start blocked on
    //     the ABSENT signal, advance logical time (which must NOT satisfy
    //     the wait), then crash mid-block. The journal must carry
    //     SignalAwaited with NO SignalSeen / ActionEmitted / Terminal.
    let (correlation, workflow_id, spec) = provision_signal_instance();
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("local").unwrap_or_else(|_| unreachable!("local is a valid node id")),
        0,
    ));
    let (crash_pre_emits, pre_resume) = match run_signal_until_crash_while_blocked(
        seed,
        &journal,
        &obs,
        &spec,
        &correlation,
        &workflow_id,
    )
    .await
    {
        Ok(out) => out,
        Err(cause) => return Some(cause),
    };
    if crash_pre_emits != 0 {
        return Some(format!(
            "crash run emitted {crash_pre_emits} Actions while blocked on the signal (expected 0)"
        ));
    }
    if !pre_resume
        .iter()
        .any(|e| matches!(e, LoadedEntry::Command(JournalCommand::SignalAwaited { .. })))
    {
        return Some(format!("crash run did not block (no SignalAwaited): {pre_resume:?}"));
    }
    if pre_resume
        .iter()
        .any(|e| matches!(e, LoadedEntry::Notification(JournalNotification::SignalSeen { .. })))
    {
        return Some(format!(
            "crash run was NOT blocked (SignalSeen present) — the signal resolved prematurely: \
             {pre_resume:?}"
        ));
    }
    if pre_resume.iter().any(|e| matches!(e, LoadedEntry::Command(JournalCommand::Terminal { .. })))
    {
        return Some(format!("crash run wrote a Terminal before the crash: {pre_resume:?}"));
    }

    // (3) Resume: re-block on the SAME signal (advance time, no signal yet),
    //     then write the signal and drive to terminal. The recorded
    //     SignalAwaited is replayed-past (no duplicate); the emit fires once.
    let (engine, clock) = provision_signal_engine(seed, Arc::clone(&journal), Arc::clone(&obs));
    let Some(mut resume_emits) = engine.take_action_emit_receiver().await else {
        return Some("resume engine had no emit receiver".to_owned());
    };
    let mut resume_sub = match obs.subscribe_all().await {
        Ok(s) => s,
        Err(err) => return Some(format!("resume subscribe_all failed: {err}")),
    };
    let _ = engine.start(&spec, &correlation, &workflow_id).await;
    // Re-block check: advance time WITHOUT the signal — must stay blocked.
    for _ in 0..4 {
        tokio::task::yield_now().await;
        clock.tick(Duration::from_millis(100));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    if drained_emit_count(&mut resume_emits).await != 0 {
        return Some(
            "resumed run emitted before the signal arrived (lost the re-block)".to_owned(),
        );
    }
    // Satisfy the signal; drive to terminal.
    drive_signal_to_terminal(&engine, &obs, &clock).await;
    let resume_fires = drained_emit_count(&mut resume_emits).await;
    if resume_fires != 1 {
        return Some(format!(
            "resumed run emitted {resume_fires} Actions (expected exactly 1 — the post-block emit)"
        ));
    }

    let Some(resumed_terminal) = drain_terminal(&mut resume_sub, &correlation).await else {
        return Some(
            "resumed signal+emit run did not reach a terminal WorkflowStatus (no bounded progress)"
                .to_owned(),
        );
    };

    // (4) K4 — replay-equivalence across signal+emit (extracted helper).
    let resumed = journal.load_journal(&workflow_id).await.unwrap_or_default();
    if let Some(cause) = assert_signal_replay_equivalence(
        &resumed,
        &uninterrupted,
        &resumed_terminal,
        &uninterrupted_terminal,
    ) {
        return Some(cause);
    }

    // (c) — notification-not-as-command cursor-advance guard (D6 / ADR-0064
    // §6; step 01-06). The resumed signal+emit trajectory carries an
    // interleaved `SignalSeen` notification. Drive its author await-points
    // through a real cursor and OBSERVE the `SignalSeen` is resolved by
    // `SignalKey` lookup OFF the positional walk, while the command-cursor
    // advances over the `SignalAwaited` + `ActionEmitted` COMMANDS only. The
    // structural guard for the trap's twin — a notification consumed as a
    // command.
    assert_notification_not_as_command(&resumed).await
}

/// K4 replay-equivalence assertion across the signal+emit shape: the
/// resumed trajectory must match the uninterrupted one in (a) entry-kind
/// sequence and (b) recorded-step contents, carry exactly ONE
/// `SignalAwaited` (the resume re-blocked on the SAME one, appending no
/// duplicate), append a `Terminal`, and reach the same terminal result.
/// Returns `None` on equivalence, `Some(cause)` on the first violation.
fn assert_signal_replay_equivalence(
    resumed: &[LoadedEntry],
    uninterrupted: &[LoadedEntry],
    resumed_terminal: &WorkflowStatus,
    uninterrupted_terminal: &WorkflowStatus,
) -> Option<String> {
    if !entry_kinds_match(resumed, uninterrupted) {
        return Some(format!(
            "resumed signal+emit entry-kind sequence differs from uninterrupted: {resumed:?} vs \
             {uninterrupted:?}"
        ));
    }
    if recorded_run_steps(resumed) != recorded_run_steps(uninterrupted) {
        return Some(format!(
            "resumed signal+emit recorded steps differ from uninterrupted: {resumed:?} vs \
             {uninterrupted:?}"
        ));
    }
    let awaited_count = resumed
        .iter()
        .filter(|e| matches!(e, LoadedEntry::Command(JournalCommand::SignalAwaited { .. })))
        .count();
    if awaited_count != 1 {
        return Some(format!(
            "resumed run has {awaited_count} SignalAwaited entries (expected 1 — re-block appends \
             no duplicate): {resumed:?}"
        ));
    }
    if !resumed.iter().any(|e| matches!(e, LoadedEntry::Command(JournalCommand::Terminal { .. }))) {
        return Some(format!("resumed signal+emit run did not append a Terminal: {resumed:?}"));
    }
    if resumed_terminal != uninterrupted_terminal {
        return Some(format!(
            "resumed signal+emit terminal {resumed_terminal:?} != uninterrupted \
             {uninterrupted_terminal:?}"
        ));
    }
    None
}

// ---------------------------------------------------------------------------
// ReplayEquivalenceProvisionRecord — slice 02 sleep extension (step 02-02)
// ---------------------------------------------------------------------------
//
// Drives `ProvisionRecordWithSleep` (the slice-02 `ctx.run → ctx.sleep →
// ctx.run` reference consumer landed in 02-01) through the SAME three-run
// crash-resume shape the slice-01 body above drives — but the crash now
// SPANS the sleep window: it lands AFTER the pre-sleep `ctx.run` records
// and the `ctx.sleep` arms its deadline, but BEFORE the post-sleep
// `ctx.run`. On resume the recorded pre-sleep `RunResult` is replayed
// (exactly-once, K1), the sleep recomputes the remaining wait from the
// recorded `deadline_unix` (an input, never a "remaining" cache; the
// post-sleep `ctx.run` fires only at/after the ORIGINAL deadline, K3), and
// the resumed trajectory is byte-identical to the uninterrupted one (K4).
// ADR-0063 §2, ADR-0064 §3/§6.

/// Two fixed addresses the `ProvisionRecordWithSleep` reference workflow's
/// pre-sleep / post-sleep `ctx.run` effects are addressed at. Distinct from
/// [`WF_TARGET`] so the sleep-shape evaluator's inboxes never collide with
/// the slice-01 single-effect evaluator's inbox.
const WF_SLEEP_PRE_TARGET: &str = "127.0.0.1:9010";
const WF_SLEEP_POST_TARGET: &str = "127.0.0.1:9011";
/// The logical wait the sleep-shape reference workflow arms via `ctx.sleep`.
const WF_SLEEP: Duration = Duration::from_secs(30);

/// Construct the instance correlation + id + spec for a
/// `ProvisionRecordWithSleep` instance. Deterministic — fixed instance id,
/// so twin runs reproduce bit-for-bit.
fn provision_sleep_instance() -> (CorrelationKey, WorkflowId, WorkflowStart) {
    let spec = ProvisionRecordWithSleep::spec();
    let correlation = CorrelationKey::derive(
        "wf-provision-sleep-inv-0001",
        &ContentHash::of(ProvisionRecordWithSleep::WORKFLOW_NAME.as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-provision-sleep-inv-0001")
        .unwrap_or_else(|_| unreachable!("wf-provision-sleep-inv-0001 is a valid instance id"));
    (correlation, workflow_id, spec)
}

/// Build a `WorkflowEngine` over the SHARED `journal` + `obs`, a fresh set
/// of `Sim*` ports, and freshly-bound pre/post transport inboxes. Returns
/// the engine's `SimClock` so the caller can advance logical time past the
/// sleep deadline (the harness owns logical time — `SimClock::sleep` parks,
/// it never auto-advances). The engine resolves `ProvisionRecordWithSleep`.
async fn provision_sleep_engine(
    seed: u64,
    journal: Arc<dyn JournalStore>,
    obs: Arc<dyn ObservationStore>,
) -> (WorkflowEngine, Arc<SimClock>, SimInbox, SimInbox) {
    let pre: SocketAddr = WF_SLEEP_PRE_TARGET
        .parse()
        .unwrap_or_else(|_| unreachable!("WF_SLEEP_PRE_TARGET is a valid socket addr"));
    let post: SocketAddr = WF_SLEEP_POST_TARGET
        .parse()
        .unwrap_or_else(|_| unreachable!("WF_SLEEP_POST_TARGET is a valid socket addr"));
    let sim_transport = SimTransport::new();
    let pre_inbox =
        sim_transport.bind_inbox(pre).await.unwrap_or_else(|_| unreachable!("bind_inbox is total"));
    let post_inbox = sim_transport
        .bind_inbox(post)
        .await
        .unwrap_or_else(|_| unreachable!("bind_inbox is total"));

    let sim_clock = Arc::new(SimClock::new());
    let transport: Arc<dyn TransportTrait> = Arc::new(sim_transport);
    let clock: Arc<dyn Clock> = sim_clock.clone();
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(seed));

    let mut registry = WorkflowRegistry::new();
    registry.register(ProvisionRecordWithSleep::spec().name, move || {
        ProvisionRecordWithSleep::new(pre, post, WF_SLEEP)
    });

    let engine = WorkflowEngine::new(journal, clock, transport, entropy, registry, obs);
    (engine, sim_clock, pre_inbox, post_inbox)
}

/// Drive the engine to terminal, advancing the `SimClock` past the sleep
/// deadline on a concurrent task so the live/replay sleep park resolves.
/// `SimClock::sleep` parks on a deadline; only `tick` wakes it — so without
/// this concurrent advance `join_all` would hang on the parked workflow.
async fn drive_sleep_to_terminal(engine: &WorkflowEngine, clock: &Arc<SimClock>) {
    let driver = Arc::clone(clock);
    let ticker = tokio::spawn(async move {
        // Advance well past the deadline in a few ticks; each `tick` wakes
        // any parked `SimClock` timer whose deadline has now passed.
        for _ in 0..8 {
            tokio::task::yield_now().await;
            driver.tick(WF_SLEEP);
        }
    });
    engine.join_all().await;
    let _ = ticker.await;
}

/// Drive a crash-injected `ProvisionRecordWithSleep` run whose crash SPANS
/// the sleep window: the pre-sleep `ctx.run` records (fires the pre effect
/// once) and the `ctx.sleep` arms its `SleepArmed` deadline and parks — then
/// the future is dropped mid-park WITHOUT advancing logical time. A
/// process-local kill DURING the sleep, modelled honestly. Returns the
/// persisted journal (pre-sleep `RunResult` + `SleepArmed`, NO post-sleep
/// run, NO `Terminal`) and the pre/post effect-fire counts of this run.
async fn run_until_crash_in_sleep(
    seed: u64,
    journal: &Arc<dyn JournalStore>,
) -> (Vec<LoadedEntry>, usize, usize) {
    let (_correlation, workflow_id, spec) = provision_sleep_instance();
    let pre: SocketAddr =
        WF_SLEEP_PRE_TARGET.parse().unwrap_or_else(|_| unreachable!("WF_SLEEP_PRE_TARGET valid"));
    let post: SocketAddr =
        WF_SLEEP_POST_TARGET.parse().unwrap_or_else(|_| unreachable!("WF_SLEEP_POST_TARGET valid"));
    let sim_transport = SimTransport::new();
    let mut pre_inbox =
        sim_transport.bind_inbox(pre).await.unwrap_or_else(|_| unreachable!("bind_inbox total"));
    let mut post_inbox =
        sim_transport.bind_inbox(post).await.unwrap_or_else(|_| unreachable!("bind_inbox total"));
    // Open the crash journal with `Started` at command-index 0 (CA-4) — the
    // same leading command `engine.start` writes on the live path — so the
    // resumed sleep-shape trajectory stays byte-identical to an
    // uninterrupted one. The raw-`ctx` `RunResult` + `SleepArmed` follow it.
    let _ = journal.append(&workflow_id, &started_entry(&spec)).await;
    {
        let cursor: Arc<dyn JournalCursor> = Arc::new(JournalCursorHandle::new(
            Arc::clone(journal),
            workflow_id.clone(),
            Vec::new(),
        ));
        let ctx = WorkflowCtx::new(
            Arc::new(SimClock::new()),
            Arc::new(sim_transport) as Arc<dyn TransportTrait>,
            Arc::new(SimEntropy::new(seed)),
            cursor,
        );
        // Pre-sleep durable step — the same `ctx.run` the author body runs.
        let pre_transport = Arc::clone(ctx.transport());
        let pre_payload = bytes::Bytes::from_static(ProvisionRecordWithSleep::FIRST_PAYLOAD);
        let _ = ctx
            .run("provision-write-pre-sleep", async move {
                pre_transport.send_datagram(pre, pre_payload).await.map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|err| Err(err.to_string()));
        // Arm the sleep + park, then "crash" mid-park: the sleep appends
        // `SleepArmed` and parks (logical time NOT advanced), then the
        // ctx + future are dropped at the end of this block.
        let sleeper = tokio::spawn(async move { ctx.sleep(WF_SLEEP).await });
        tokio::task::yield_now().await;
        sleeper.abort();
        let _ = sleeper.await;
    }
    let pre_fires = delivered_count(&mut pre_inbox).await;
    let post_fires = delivered_count(&mut post_inbox).await;
    let trajectory = journal.load_journal(&workflow_id).await.unwrap_or_default();
    (trajectory, pre_fires, post_fires)
}

/// Drive an uninterrupted `ProvisionRecordWithSleep` run to terminal through
/// the engine; return the loaded journal trajectory + terminal result.
async fn run_sleep_uninterrupted(seed: u64) -> (Vec<LoadedEntry>, Option<WorkflowStatus>) {
    let (correlation, workflow_id, spec) = provision_sleep_instance();
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("local").unwrap_or_else(|_| unreachable!("local is a valid node id")),
        0,
    ));
    let (engine, clock, _pre, _post) =
        provision_sleep_engine(seed, Arc::clone(&journal), Arc::clone(&obs)).await;
    let Ok(mut subscription) = obs.subscribe_all().await else {
        return (Vec::new(), None);
    };
    let _ = engine.start(&spec, &correlation, &workflow_id).await;
    drive_sleep_to_terminal(&engine, &clock).await;
    let trajectory = journal.load_journal(&workflow_id).await.unwrap_or_default();
    let terminal = drain_terminal(&mut subscription, &correlation).await;
    (trajectory, terminal)
}

/// Slice-02 sleep extension of `ReplayEquivalenceProvisionRecord` (K4).
///
/// Drives the three-run crash-resume shape over the `ctx.run → ctx.sleep →
/// ctx.run` workflow, with the crash SPANNING the sleep window. Returns
/// `None` on success, or `Some(cause)` naming the first violated property.
async fn check_replay_equivalence_across_sleep(seed: u64) -> Option<String> {
    // (1) Uninterrupted run — the reference trajectory across the sleep.
    let (uninterrupted, uninterrupted_terminal) = run_sleep_uninterrupted(seed).await;
    let uninterrupted_terminal = uninterrupted_terminal?;
    if !uninterrupted
        .iter()
        .any(|e| matches!(e, LoadedEntry::Command(JournalCommand::SleepArmed { .. })))
    {
        return Some(format!(
            "uninterrupted sleep run recorded no SleepArmed entry: {uninterrupted:?}"
        ));
    }

    // (2) Crash-injected run on a fresh shared journal — pre-sleep RunResult
    //     + SleepArmed, no post-sleep run, no Terminal. The pre-sleep effect
    //     fires exactly once; the post-sleep effect never fires (the crash
    //     is DURING the sleep).
    let (_correlation, workflow_id, spec) = provision_sleep_instance();
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let (pre_resume, crash_pre_fires, crash_post_fires) =
        run_until_crash_in_sleep(seed, &journal).await;
    if crash_pre_fires != 1 {
        return Some(format!(
            "crash run fired the pre-sleep effect {crash_pre_fires} times (expected 1)"
        ));
    }
    if crash_post_fires != 0 {
        return Some(format!(
            "crash run fired the post-sleep effect {crash_post_fires} times (the crash spans the \
             sleep — it must be 0)"
        ));
    }
    if let Some(cause) = crash_journal_spans_sleep_shape(&pre_resume) {
        return Some(cause);
    }

    // (3) Resume from the persisted journal through the engine. The recorded
    //     pre-sleep RunResult is replayed (NO re-fire, K1); the sleep
    //     recomputes the remaining wait from the recorded deadline; the
    //     post-sleep run fires once (K3); the run reaches terminal.
    let (correlation, _wid, _spec2) = provision_sleep_instance();
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("local").unwrap_or_else(|_| unreachable!("local is a valid node id")),
        0,
    ));
    let (engine, clock, mut resume_pre_inbox, mut resume_post_inbox) =
        provision_sleep_engine(seed, Arc::clone(&journal), Arc::clone(&obs)).await;
    let mut resume_sub = match obs.subscribe_all().await {
        Ok(s) => s,
        Err(err) => return Some(format!("resume subscribe_all failed: {err}")),
    };
    let _ = engine.start(&spec, &correlation, &workflow_id).await;
    drive_sleep_to_terminal(&engine, &clock).await;

    // K1 — the pre-sleep `ctx.run` is replayed on resume: ZERO re-fires.
    let resume_pre_fires = delivered_count(&mut resume_pre_inbox).await;
    if resume_pre_fires != 0 {
        return Some(format!(
            "resume re-fired the recorded pre-sleep effect {resume_pre_fires} times (must be 0)"
        ));
    }
    // The post-sleep `ctx.run` was never recorded pre-crash → it is live on
    // resume and fires exactly once at/after the original deadline (K3).
    let resume_post_fires = delivered_count(&mut resume_post_inbox).await;
    if resume_post_fires != 1 {
        return Some(format!(
            "post-sleep effect fired {resume_post_fires} times on resume (expected exactly 1)"
        ));
    }

    let Some(resumed_terminal) = drain_terminal(&mut resume_sub, &correlation).await else {
        return Some(
            "resumed sleep run did not reach a terminal WorkflowStatus (no bounded progress)"
                .to_owned(),
        );
    };

    // K4 — replay-equivalence across the sleep. The resumed trajectory must
    // match the uninterrupted one in (a) the ordered sequence of entry
    // KINDS and (b) the deterministic recorded-step CONTENTS (each
    // `RunResult`'s name + result bytes). The clock-derived `SleepArmed
    // { deadline_unix }` is an INPUT computed from the live clock, so its
    // absolute value differs between two independently-constructed
    // `SimClock`s (each captures its own wall-clock `unix_epoch`); requiring
    // it to be byte-identical across the uninterrupted vs resumed clocks
    // would be asserting clock-epoch identity, not replay-equivalence. The
    // load-bearing replay property is instead: the resumed run REPLAYS the
    // crash run's recorded `SleepArmed` (it does NOT re-arm a fresh
    // deadline) — proven by the resumed trajectory carrying exactly the
    // crash run's recorded deadline.
    let resumed = journal.load_journal(&workflow_id).await.unwrap_or_default();
    if !entry_kinds_match(&resumed, &uninterrupted) {
        return Some(format!(
            "resumed sleep entry-kind sequence differs from uninterrupted: {resumed:?} vs \
             {uninterrupted:?}"
        ));
    }
    if recorded_run_steps(&resumed) != recorded_run_steps(&uninterrupted) {
        return Some(format!(
            "resumed sleep recorded RunResult steps differ from uninterrupted: {resumed:?} vs \
             {uninterrupted:?}"
        ));
    }
    // The resumed SleepArmed deadline is the crash run's recorded one
    // (replay re-armed nothing) — pre_resume is the crash journal.
    if sleep_armed_deadline(&resumed) != sleep_armed_deadline(&pre_resume) {
        return Some(format!(
            "resumed run re-armed the sleep instead of replaying the recorded deadline: \
             resumed={resumed:?} crash={pre_resume:?}"
        ));
    }
    if !resumed.iter().any(|e| matches!(e, LoadedEntry::Command(JournalCommand::Terminal { .. }))) {
        return Some(format!("resumed sleep run did not append a Terminal entry: {resumed:?}"));
    }
    if resumed_terminal != uninterrupted_terminal {
        return Some(format!(
            "resumed sleep terminal {resumed_terminal:?} != uninterrupted {uninterrupted_terminal:?}"
        ));
    }

    None
}

/// Assert the crash-injected sleep run's persisted journal has the
/// pre-sleep crash shape: a recorded pre-sleep `RunResult` AND a
/// `SleepArmed` (the crash spanned the sleep window) AND NO `Terminal`
/// (the crash landed before terminal). Returns `Some(cause)` on the first
/// violation, `None` when all three hold.
fn crash_journal_spans_sleep_shape(pre_resume: &[LoadedEntry]) -> Option<String> {
    if !pre_resume
        .iter()
        .any(|e| matches!(e, LoadedEntry::Command(JournalCommand::RunResult { .. })))
    {
        return Some(format!("crash run left no recorded pre-sleep RunResult: {pre_resume:?}"));
    }
    if !pre_resume
        .iter()
        .any(|e| matches!(e, LoadedEntry::Command(JournalCommand::SleepArmed { .. })))
    {
        return Some(format!("crash run did not span the sleep (no SleepArmed): {pre_resume:?}"));
    }
    if pre_resume.iter().any(|e| matches!(e, LoadedEntry::Command(JournalCommand::Terminal { .. })))
    {
        return Some(format!("crash run wrote a Terminal before the crash: {pre_resume:?}"));
    }
    None
}

/// The stable kind label of a single [`JournalCommand`] — the per-variant
/// projection both [`entry_kinds`] and [`command_kinds`] share so the six
/// command arms live in exactly ONE place (they drifted as two verbatim
/// copies before). Mirrors the production `command_kind` determinism-gate
/// label (`workflow_runtime`), but is defined here independently: that one
/// is crate-private to the control-plane and serves the fail-closed gate;
/// this serves the sim replay-equivalence oracle.
const fn journal_command_kind(command: &JournalCommand) -> &'static str {
    match command {
        JournalCommand::Started { .. } => "Started",
        JournalCommand::RunResult { .. } => "RunResult",
        JournalCommand::SleepArmed { .. } => "SleepArmed",
        JournalCommand::SignalAwaited { .. } => "SignalAwaited",
        JournalCommand::ActionEmitted { .. } => "ActionEmitted",
        JournalCommand::RetryAttempted { .. } => "RetryAttempted",
        JournalCommand::Terminal { .. } => "Terminal",
    }
}

/// The ordered sequence of journal-entry KINDS — the replay-equivalence
/// shape oracle that ignores clock-derived absolute values (a `SleepArmed`
/// deadline is computed from the live clock's wall-clock epoch).
///
/// Classifies the typed [`LoadedEntry`] boundary sum (D1): commands and
/// notifications interleave in one ordered run; the kind string names the
/// inner variant regardless of class — `SignalSeen` is a notification,
/// every other kind is a command (delegated to [`journal_command_kind`]).
fn entry_kinds(run: &[LoadedEntry]) -> Vec<&'static str> {
    run.iter()
        .map(|e| match e {
            LoadedEntry::Command(command) => journal_command_kind(command),
            LoadedEntry::Notification(JournalNotification::SignalSeen { .. }) => "SignalSeen",
        })
        .collect()
}

/// Whether two runs have the same ordered sequence of entry kinds.
fn entry_kinds_match(a: &[LoadedEntry], b: &[LoadedEntry]) -> bool {
    entry_kinds(a) == entry_kinds(b)
}

/// The ordered command-kind sequence of `run` — the kinds of the
/// `LoadedEntry::Command`s ONLY, in append order (step 01-06, D6 / ADR-0064
/// §6). This is the positional command walk the cursor consumes: the
/// `Vec<JournalCommand>` partition `partition_loaded_run` produces.
/// Notifications (`SignalSeen`) are OFF the walk and are excluded — they are
/// `SignalKey`-correlated, never walked as a command (D2). A command-cursor
/// advances by exactly 1 per command and ZERO per notification, so the
/// length of this sequence IS the total command-cursor advance count for a
/// full replay of `run`.
fn command_kinds(run: &[LoadedEntry]) -> Vec<&'static str> {
    run.iter()
        .filter_map(|e| match e {
            LoadedEntry::Command(command) => Some(journal_command_kind(command)),
            LoadedEntry::Notification(JournalNotification::SignalSeen { .. }) => None,
        })
        .collect()
}

/// The number of `LoadedEntry::Notification`s in `run` — the
/// `SignalKey`-correlated entries that live OFF the positional command walk
/// (step 01-06, D6). These NEVER advance the command-cursor; the
/// [`assert_notification_not_as_command`] guard requires ≥1 so the probe
/// exercises the notification path rather than a notification-free run.
fn notification_count(run: &[LoadedEntry]) -> usize {
    run.iter().filter(|e| matches!(e, LoadedEntry::Notification(_))).count()
}

/// (b) — Started-at-command-index-0 full-command-sequence equality (D6 /
/// ADR-0064 §6; step 01-06). The structural regression guard that would
/// have caught the trap: a dropped `Started` write, or a divergent command
/// sequence between the resumed and uninterrupted runs, fails this.
///
/// Widens the slice-01 "recorded `RunResult` matches" equality to a FULL
/// command-kind-sequence equality that pins `Started` at command-index 0 in
/// BOTH runs. Returns `None` on equivalence, `Some(cause)` naming the first
/// violation:
///
/// - either run's command sequence does NOT begin with `Started` at index 0
///   (the trap: `WorkflowEngine::start` failing to write the `Started`
///   command), or
/// - the two command-kind sequences diverge (a non-byte-identical replay).
fn assert_started_at_index_0_and_command_sequence_identical(
    resumed: &[LoadedEntry],
    uninterrupted: &[LoadedEntry],
) -> Option<String> {
    let resumed_kinds = command_kinds(resumed);
    let uninterrupted_kinds = command_kinds(uninterrupted);
    if resumed_kinds.first() != Some(&"Started") {
        return Some(format!(
            "resumed command sequence does not begin with `Started` at command-index 0 \
             (the trap — a dropped Started write): {resumed:?}"
        ));
    }
    if uninterrupted_kinds.first() != Some(&"Started") {
        return Some(format!(
            "uninterrupted command sequence does not begin with `Started` at command-index 0 \
             (the trap — a dropped Started write): {uninterrupted:?}"
        ));
    }
    if resumed_kinds != uninterrupted_kinds {
        return Some(format!(
            "resumed command-kind sequence diverges from uninterrupted (not byte-identical): \
             {resumed_kinds:?} vs {uninterrupted_kinds:?}"
        ));
    }
    None
}

/// The `SignalKey` of the first `SignalAwaited` command in `run`, if any —
/// the key the signal+emit author await replays its `ctx.wait_for_signal`
/// against (step 01-06).
fn first_awaited_signal_key(run: &[LoadedEntry]) -> Option<overdrive_core::workflow::SignalKey> {
    run.iter().find_map(|e| match e {
        LoadedEntry::Command(JournalCommand::SignalAwaited { signal_key }) => {
            Some(signal_key.clone())
        }
        _ => None,
    })
}

/// (c) — notification-not-as-command cursor-advance guard (D6 / ADR-0064 §6;
/// step 01-06). The structural regression guard for the trap's twin: a
/// `SignalSeen` notification entering the positional command walk.
///
/// Drives the signal+emit `run`'s author await-points through a REAL
/// [`JournalCursorHandle`] — the SAME `partition_loaded_run` the production
/// engine uses — and OBSERVES that the `SignalSeen` notification is resolved
/// by `SignalKey` lookup OFF the positional command walk, while the
/// command-cursor advances over the `SignalAwaited` and `ActionEmitted`
/// COMMANDS only:
///
/// 1. `replay_signal(key)` — the `ctx.wait_for_signal` await replays: the
///    cursor points at the `SignalAwaited` command, resolves the recorded
///    `SignalSeen` value by KEY (never by position), and advances the
///    command-cursor by EXACTLY 1 (past `SignalAwaited` only — the
///    notification is off the walk). A replay hit returns `Ok(Some(value))`.
/// 2. `replay_emit()` — the subsequent `ctx.emit_action` await replays: the
///    cursor now points at the `ActionEmitted` command and returns
///    `Ok(true)`. **This is the load-bearing observation:** if the
///    `SignalSeen` had been walked as a COMMAND, step 1's single advance
///    would have landed the cursor ON the `SignalSeen`, and step 2 would
///    trip the Layer-1 type-at-index gate (`NonDeterministic`, expected
///    `ActionEmitted`, actual `SignalSeen`) — `Err`, NOT `Ok(true)`.
///
/// The guard requires the run to carry an interleaved notification AND a
/// `SignalAwaited` + `ActionEmitted` command pair (the signal+emit shape).
/// Returns `None` when the notification stays off the walk and both await
/// replays hit, `Some(cause)` otherwise.
async fn assert_notification_not_as_command(run: &[LoadedEntry]) -> Option<String> {
    if notification_count(run) == 0 {
        return Some(format!(
            "notification-not-as-command guard requires a run with an interleaved SignalSeen \
             notification, but `run` carries none: {run:?}"
        ));
    }
    let Some(signal_key) = first_awaited_signal_key(run) else {
        return Some(format!(
            "notification-not-as-command guard requires a SignalAwaited command in `run`: {run:?}"
        ));
    };

    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let workflow_id = WorkflowId::new("wf-notif-not-cmd-probe-0001")
        .unwrap_or_else(|_| unreachable!("wf-notif-not-cmd-probe-0001 is a valid instance id"));
    let cursor = JournalCursorHandle::new(journal, workflow_id, run.to_vec());

    // (1) The `ctx.wait_for_signal` await replays: SignalSeen resolved by
    //     KEY (off the walk), command-cursor advances by exactly 1 past the
    //     SignalAwaited command.
    match cursor.replay_signal(&signal_key).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Some(format!(
                "replay_signal did not resolve the recorded SignalSeen by key — the notification \
                 was not in the off-the-walk lookup map (consumed as a command?): {run:?}"
            ));
        }
        Err(err) => {
            return Some(format!(
                "replay_signal tripped the determinism gate at the SignalAwaited cursor \
                 ({err:?}) — a notification leaked into the command walk: {run:?}"
            ));
        }
    }

    // (2) The subsequent `ctx.emit_action` await replays against the
    //     ActionEmitted COMMAND — NOT against the SignalSeen. A notification
    //     walked as a command would have landed the cursor ON the SignalSeen
    //     here, tripping the Layer-1 gate (Err), not returning Ok(true).
    match cursor.replay_emit().await {
        Ok(true) => None,
        Ok(false) => Some(format!(
            "replay_emit fell to the live path after replay_signal — the command-cursor \
             over-advanced (a notification consumed as a command displaced ActionEmitted): {run:?}"
        )),
        Err(err) => Some(format!(
            "replay_emit tripped the determinism gate after replay_signal ({err:?}) — the \
             command-cursor landed on the SignalSeen notification (consumed as a command, the \
             trap's twin): {run:?}"
        )),
    }
}

/// The deterministic recorded-step contents — each `RunResult`'s name +
/// result bytes (clock-independent, so byte-comparable across runs).
fn recorded_run_steps(run: &[LoadedEntry]) -> Vec<(String, Vec<u8>)> {
    run.iter()
        .filter_map(|e| match e {
            LoadedEntry::Command(JournalCommand::RunResult { name, result_bytes, .. }) => {
                Some((name.clone(), result_bytes.clone()))
            }
            _ => None,
        })
        .collect()
}

/// The recorded `SleepArmed` deadline in a run, if any.
fn sleep_armed_deadline(run: &[LoadedEntry]) -> Option<Duration> {
    run.iter().find_map(|e| match e {
        LoadedEntry::Command(JournalCommand::SleepArmed { deadline_unix, .. }) => {
            Some(*deadline_unix)
        }
        _ => None,
    })
}

// ---------------------------------------------------------------------------
// WorkflowJournalWriteOrdering (workflow-primitive step 01-07)
// ---------------------------------------------------------------------------

/// Evaluate `WorkflowJournalWriteOrdering` (ADR-0064 §6).
///
/// Mirrors ADR-0035 `WriteThroughOrdering`. Under an injected
/// fsync-failure, the live-path `ctx.run` record FAILS, the cursor does
/// NOT advance (a retry is still a LIVE call), and the journal carries no
/// phantom entry.
#[must_use]
pub async fn evaluate_workflow_journal_write_ordering(seed: u64) -> InvariantResult {
    let name = "workflow-journal-write-ordering";
    let fail = |cause: String| result(name, InvariantStatus::Fail, "host-0", Some(cause));

    let store = Arc::new(SimJournalStore::new());
    let journal: Arc<dyn JournalStore> = Arc::clone(&store) as Arc<dyn JournalStore>;
    let workflow_id =
        WorkflowId::new("wf-ordering-0001").unwrap_or_else(|_| unreachable!("valid id"));
    let target: SocketAddr = WF_TARGET.parse().unwrap_or_else(|_| unreachable!("WF_TARGET valid"));

    let sim_transport = SimTransport::new();
    let mut inbox =
        sim_transport.bind_inbox(target).await.unwrap_or_else(|_| unreachable!("bind_inbox total"));

    let cursor: Arc<dyn JournalCursor> =
        Arc::new(JournalCursorHandle::new(Arc::clone(&journal), workflow_id.clone(), Vec::new()));
    let ctx = WorkflowCtx::new(
        Arc::new(SimClock::new()),
        Arc::new(sim_transport) as Arc<dyn TransportTrait>,
        Arc::new(SimEntropy::new(seed)),
        Arc::clone(&cursor),
    );
    // The provision-write `ctx.run` durable step; the raw ctx result is
    // observed so the injected fsync failure surfaces as a JournalRecord
    // error (the success type `T` is `Result<usize, String>`, so a
    // record failure is distinguishable from an effect failure).
    let run_step = || {
        let transport = Arc::clone(ctx.transport());
        let payload = bytes::Bytes::from_static(ProvisionRecord::PAYLOAD);
        ctx.run("provision-write", async move {
            transport.send_datagram(target, payload).await.map_err(|e| e.to_string())
        })
    };

    // Arm the fsync failure — the next live record (append) fails.
    store.inject_fsync_failure();
    match run_step().await {
        Ok(_) => return fail("live record succeeded under injected fsync failure".to_owned()),
        Err(_err) => {}
    }

    // No phantom entry.
    let after_fail = journal.load_journal(&workflow_id).await.unwrap_or_default();
    if !after_fail.is_empty() {
        return fail(format!("failed fsync left a phantom entry: {after_fail:?}"));
    }

    // Cursor did NOT advance: clear the failure and retry through the SAME
    // cursor. The load-bearing observable is that the retry is a LIVE call
    // (cursor stayed at step 0 over an empty buffer), NOT a replay. A
    // replay would fire ZERO datagrams and record nothing; a live call
    // fires the transport effect and records exactly one journal entry.
    //
    // The transport effect is at-least-once by design (ADR-0064: the
    // failed live-path call ALREADY fired its datagram before the append
    // failed; exactly-once is the replay/resume guarantee, not the
    // within-boot retry guarantee). So the failed call + the retry deliver
    // TWO datagrams total — and that the retry contributed a fresh live
    // fire (the count rose past the single pre-clear fire) is the proof
    // the cursor did not advance into a spurious replay.
    let fires_before_retry = delivered_count(&mut inbox).await;
    store.clear_fsync_failure();
    match run_step().await {
        Ok(Ok(bytes_sent)) if bytes_sent == ProvisionRecord::PAYLOAD.len() => {}
        Ok(Ok(bytes_sent)) => {
            return fail(format!("retry response had wrong byte count: {bytes_sent}"));
        }
        Ok(Err(effect_err)) => {
            return fail(format!("retry effect failed: {effect_err}"));
        }
        Err(err) => return fail(format!("retry after clear failed: {err}")),
    }
    let fires_from_retry = delivered_count(&mut inbox).await;
    if fires_from_retry != 1 {
        return fail(format!(
            "retry was not a single LIVE fire: it delivered {fires_from_retry} datagrams \
             (expected exactly 1 — a replay would deliver 0, proving the cursor wrongly advanced)"
        ));
    }
    // The pre-clear failed call fired at-least-once (its datagram landed
    // before the append failed); the retry then fired exactly once more.
    if fires_before_retry != 1 {
        return fail(format!(
            "the failed live-path call should have fired its datagram once before the append \
             failed (at-least-once transport); observed {fires_before_retry}"
        ));
    }
    let after_clear = journal.load_journal(&workflow_id).await.unwrap_or_default();
    if after_clear.len() != 1 {
        return fail(format!("expected exactly one recorded entry after retry: {after_clear:?}"));
    }

    result(name, InvariantStatus::Pass, "host-0", None)
}

// ---------------------------------------------------------------------------
// WorkflowExactlyOnceEffectOnResume (workflow-primitive step 01-07, K1)
// ---------------------------------------------------------------------------

/// Evaluate `WorkflowExactlyOnceEffectOnResume` (ADR-0064 §6; US-WP-3 AC1 / K1).
///
/// Crash after `ctx.run` records → resume → the recorded effect is
/// replayed WITHOUT re-firing the transport (zero datagrams on the resumed
/// boot's inbox) and the run reaches terminal.
#[must_use]
pub async fn evaluate_workflow_exactly_once_effect_on_resume(seed: u64) -> InvariantResult {
    let name = "workflow-exactly-once-effect-on-resume";
    let fail = |cause: String| result(name, InvariantStatus::Fail, "host-0", Some(cause));

    let (correlation, workflow_id, spec) = provision_instance();
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());

    // Crash run: fire + record step 0, no terminal.
    let (pre_resume, crash_fires) = run_until_crash(seed, &journal).await;
    if crash_fires != 1 {
        return fail(format!("pre-crash run fired the effect {crash_fires} times (expected 1)"));
    }
    if !pre_resume
        .iter()
        .any(|e| matches!(e, LoadedEntry::Command(JournalCommand::RunResult { .. })))
    {
        return fail(format!("crash run recorded no RunResult: {pre_resume:?}"));
    }

    // Resume: zero additional fires, reach terminal.
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("local").unwrap_or_else(|_| unreachable!("local valid")),
        0,
    ));
    let (engine, mut inbox) = provision_engine(seed, Arc::clone(&journal), Arc::clone(&obs)).await;
    // Subscribe BEFORE driving — the terminal row is broadcast live.
    let mut subscription = match obs.subscribe_all().await {
        Ok(s) => s,
        Err(err) => return fail(format!("resume subscribe_all failed: {err}")),
    };
    let _ = engine.start(&spec, &correlation, &workflow_id).await;
    engine.join_all().await;

    let resume_fires = delivered_count(&mut inbox).await;
    if resume_fires != 0 {
        return fail(format!(
            "resume re-fired the recorded effect {resume_fires} times (exactly-once violated)"
        ));
    }
    if drain_terminal(&mut subscription, &correlation).await.is_none() {
        return fail("resumed run did not reach terminal".to_owned());
    }

    result(name, InvariantStatus::Pass, "host-0", None)
}

// ---------------------------------------------------------------------------
// WorkflowTerminalStatusProjection (workflow-result-error-model step 02-01)
// ---------------------------------------------------------------------------
//
// Pins the engine-owned body-`Result` → `WorkflowStatus` projection (ADR-0065
// D3) as a structural DST property: a workflow whose body returns
// `Err(TerminalError::explicit(detail))` MUST project to
// `WorkflowStatus::Failed { terminal }` (NOT `Completed`, NOT a contentless
// terminal — the contentless terminal enum was deleted in step 01-03) carrying
// the SAME `kind` + `detail` the body authored. The terminal is the engine's
// projection, distinct from the body return type (the crux of the ADR-0065
// research finding). The invariant also pins the D3 lossless-projection
// contract: the durable terminal (`JournalCommand::Terminal { status }`) and
// the observable terminal (`ObservationRow::WorkflowTerminal { status }`) carry
// byte-identical `WorkflowStatus` bytes.

/// The detail the always-failing reference workflow authors via
/// `TerminalError::explicit`. A fixed, deterministic string so the projected
/// terminal replays bit-identically across seeds (the `detail` is an INPUT,
/// not engine-derived state — `development.md` § "Persist inputs, not derived
/// state").
const WF_TERMINAL_DETAIL: &str = "authored terminal failure (step 02-01 D3)";

/// The instance address bound for the always-failing workflow's engine. Never
/// receives a datagram (the body returns before any `ctx.run`), but a bound
/// inbox keeps the engine builder shape identical to the slice-01 builder.
const WF_FAILURE_TARGET: &str = "127.0.0.1:9020";

/// A minimal reference workflow whose body returns
/// `Err(TerminalError::explicit(WF_TERMINAL_DETAIL))` UNCONDITIONALLY — the
/// thinnest authored-failure shape. Distinct from the slice-01/02/03 fixtures
/// (which only fail when a `ctx` effect fails); this one always fails, so the
/// engine's `Err(TerminalError)` → `WorkflowStatus::Failed` projection is
/// driven deterministically with no seed-dependent branch.
struct AlwaysExplicitFailure;

impl AlwaysExplicitFailure {
    /// The workflow name this fixture registers under. Kebab-case, matching
    /// the `WorkflowName` grammar.
    const WORKFLOW_NAME: &'static str = "always-explicit-failure";

    /// The concrete [`WorkflowStart`] this fixture corresponds to. Takes a
    /// unit `Input`, so the opaque CBOR `input` is the 1-byte encoding of
    /// `()` the `ErasedWorkflowAdapter` decodes back before calling `run`.
    fn spec() -> WorkflowStart {
        let mut input: Vec<u8> = Vec::new();
        ciborium::into_writer(&(), &mut input)
            .unwrap_or_else(|_| unreachable!("CBOR-encoding the unit type is total"));
        WorkflowStart {
            name: WorkflowName::new(Self::WORKFLOW_NAME)
                .unwrap_or_else(|_| unreachable!("WORKFLOW_NAME is a valid kebab constant")),
            input,
        }
    }
}

#[async_trait::async_trait]
impl Workflow for AlwaysExplicitFailure {
    type Output = ();
    type Input = ();

    async fn run(&self, _ctx: &WorkflowCtx, _input: ()) -> Result<(), TerminalError> {
        // Unconditional authored terminal failure — the body never reaches a
        // `ctx` await. The engine projects this `Err` to
        // `WorkflowStatus::Failed { terminal }` (ADR-0065 §3).
        Err(TerminalError::explicit(WF_TERMINAL_DETAIL))
    }
}

/// Construct the instance correlation + id + spec for an
/// `AlwaysExplicitFailure` instance. Deterministic — fixed instance id, so
/// twin runs reproduce bit-for-bit.
fn failure_instance() -> (CorrelationKey, WorkflowId, WorkflowStart) {
    let spec = AlwaysExplicitFailure::spec();
    let correlation = CorrelationKey::derive(
        "wf-terminal-projection-0001",
        &ContentHash::of(AlwaysExplicitFailure::WORKFLOW_NAME.as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-terminal-projection-0001")
        .unwrap_or_else(|_| unreachable!("wf-terminal-projection-0001 is a valid instance id"));
    (correlation, workflow_id, spec)
}

/// Build a `WorkflowEngine` over the SHARED `journal` + `obs`, a fresh set of
/// `Sim*` ports, and a freshly-bound transport inbox, resolving
/// `AlwaysExplicitFailure`.
async fn failure_engine(
    seed: u64,
    journal: Arc<dyn JournalStore>,
    obs: Arc<dyn ObservationStore>,
) -> WorkflowEngine {
    let target: SocketAddr = WF_FAILURE_TARGET
        .parse()
        .unwrap_or_else(|_| unreachable!("WF_FAILURE_TARGET is a valid socket addr"));
    let sim_transport = SimTransport::new();
    // Bind the inbox so the transport has a registered endpoint, matching the
    // slice-01 builder shape; the always-failing body never sends to it.
    let _inbox = sim_transport
        .bind_inbox(target)
        .await
        .unwrap_or_else(|_| unreachable!("SimTransport::bind_inbox is total"));

    let transport: Arc<dyn TransportTrait> = Arc::new(sim_transport);
    let clock: Arc<dyn Clock> = Arc::new(SimClock::new());
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(seed));

    let mut registry = WorkflowRegistry::new();
    registry.register(AlwaysExplicitFailure::spec().name, || AlwaysExplicitFailure);

    WorkflowEngine::new(journal, clock, transport, entropy, registry, obs)
}

/// Extract the `WorkflowStatus` from the single `JournalCommand::Terminal`
/// entry in `run`, if present. The engine appends exactly one `Terminal` on
/// the body's terminal (uninterrupted run, no resume).
fn journal_terminal_status(run: &[LoadedEntry]) -> Option<&WorkflowStatus> {
    run.iter().find_map(|e| match e {
        LoadedEntry::Command(JournalCommand::Terminal { status }) => Some(status),
        _ => None,
    })
}

/// CBOR-encode a `WorkflowStatus` to its canonical bytes — the durable wire
/// form. Used to assert the journal terminal and the observation terminal
/// carry byte-identical `WorkflowStatus` payloads (D3 lossless projection).
fn status_bytes(status: &WorkflowStatus) -> Vec<u8> {
    let mut bytes: Vec<u8> = Vec::new();
    ciborium::into_writer(status, &mut bytes)
        .unwrap_or_else(|_| unreachable!("CBOR-encoding WorkflowStatus is total"));
    bytes
}

/// Evaluate `WorkflowTerminalStatusProjection` (ADR-0065 §3, D3).
///
/// Drives an `AlwaysExplicitFailure` workflow (body returns
/// `Err(TerminalError::explicit(detail))`) through the real `WorkflowEngine` +
/// `SimJournalStore`, then asserts:
///   1. the engine projected the body's `Err` to `WorkflowStatus::Failed {
///      terminal }` (NOT `Completed`, NOT contentless) with `terminal.kind()
///      == Explicit` and `terminal.detail() == WF_TERMINAL_DETAIL` (AC2);
///   2. the SAME `WorkflowStatus` round-trips byte-equal through BOTH the
///      durable journal `Terminal { status }` AND the observable
///      `WorkflowTerminal { status }` obs row (AC3 — D3 lossless projection).
#[must_use]
pub async fn evaluate_workflow_terminal_status_projection(seed: u64) -> InvariantResult {
    let name = "workflow-terminal-status-projection";
    // The seed is echoed by the harness's RunReport on failure; embedding it
    // in the cause string makes a `cargo dst --seed <N>` reproduction
    // self-describing from the failure line alone.
    let fail = |cause: String| {
        result(name, InvariantStatus::Fail, "host-0", Some(format!("[seed={seed}] {cause}")))
    };

    let (correlation, workflow_id, spec) = failure_instance();
    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("local").unwrap_or_else(|_| unreachable!("local is a valid node id")),
        0,
    ));
    let engine = failure_engine(seed, Arc::clone(&journal), Arc::clone(&obs)).await;
    // Subscribe BEFORE driving — the `WorkflowTerminal` row is broadcast live
    // (never snapshotted), so a post-run subscriber would miss it.
    let mut subscription = match obs.subscribe_all().await {
        Ok(s) => s,
        Err(err) => return fail(format!("subscribe_all failed: {err}")),
    };
    let _ = engine.start(&spec, &correlation, &workflow_id).await;
    engine.join_all().await;

    // (1) The observable terminal — drained off the live broadcast.
    let Some(obs_status) = drain_terminal(&mut subscription, &correlation).await else {
        return fail(
            "engine wrote no WorkflowTerminal observation row for the authored-failure run"
                .to_owned(),
        );
    };

    // AC2 — the body's `Err(TerminalError::explicit)` projected to
    // `Failed { terminal }` (NOT Completed, NOT any other variant), carrying
    // the SAME kind + detail the body authored.
    let WorkflowStatus::Failed { terminal: obs_terminal } = &obs_status else {
        return fail(format!(
            "expected WorkflowStatus::Failed for an authored Err(TerminalError::explicit), got \
             {obs_status:?}"
        ));
    };
    if obs_terminal.kind() != TerminalErrorKind::Explicit {
        return fail(format!(
            "expected TerminalErrorKind::Explicit, got {:?}",
            obs_terminal.kind()
        ));
    }
    if obs_terminal.detail() != WF_TERMINAL_DETAIL {
        return fail(format!(
            "terminal detail mismatch: expected {WF_TERMINAL_DETAIL:?}, got {:?}",
            obs_terminal.detail()
        ));
    }

    // (2) The durable terminal — loaded from the journal.
    let run = journal.load_journal(&workflow_id).await.unwrap_or_default();
    let Some(journal_status) = journal_terminal_status(&run) else {
        return fail(format!("journal carries no Terminal command after the run: {run:?}"));
    };

    // AC3 — D3 lossless projection: the durable terminal and the observable
    // terminal carry byte-identical `WorkflowStatus` bytes. Structural
    // equality first (a clearer failure message), then the byte-equal
    // round-trip the AC names explicitly.
    if journal_status != &obs_status {
        return fail(format!(
            "journal Terminal status {journal_status:?} != observation terminal status \
             {obs_status:?} (lossy projection)"
        ));
    }
    let journal_bytes = status_bytes(journal_status);
    let obs_bytes = status_bytes(&obs_status);
    if journal_bytes != obs_bytes {
        return fail(format!(
            "journal Terminal status bytes ({} bytes) differ from observation terminal status \
             bytes ({} bytes) — D3 lossless projection violated",
            journal_bytes.len(),
            obs_bytes.len(),
        ));
    }

    result(name, InvariantStatus::Pass, "host-0", None)
}

// ---------------------------------------------------------------------------
// WorkflowBudgetExhaustionMintsTerminal (workflow-result-error-model step 04-02)
// ---------------------------------------------------------------------------
//
// The DST sibling of NEW-5 (the example-based acceptance at
// `crates/overdrive-control-plane/tests/acceptance/workflow_budget_exhaustion_mints_terminal.rs`)
// and the DST counterpart to `WorkflowTerminalStatusProjection`. Pins the
// ADR-0065 §D4 engine-owns-retry / body-owns-only-terminal split as a forever
// property over the harness trajectory: a workflow whose body ALWAYS fails
// transiently is re-driven by the engine up to the engine-constant
// `WORKFLOW_RETRY_BUDGET`, then the engine MINTS `WorkflowStatus::Failed {
// terminal: BudgetExhausted }`. The body authors NO failure of its own — every
// drive returns a transient the engine absorbs and re-drives; the
// `BudgetExhausted` terminal is engine-minted (D4).
//
// The invariant asserts the OUTCOME (budget → engine-minted
// `Failed{BudgetExhausted}`; body authored no terminal), which holds
// regardless of HOW the transient is signalled — so when the Phase-4 review
// (Option A, ADR-0065 §2 fidelity) moved the transient channel from a
// since-deleted body-return retryable kind to a `ctx.run_retryable` step, this
// invariant stayed green: the engine-minted-terminal contract is unchanged.

/// The instance address bound for the always-transient workflow's engine.
/// Never receives a datagram (the body returns the transient before any
/// `ctx` send), but a bound inbox keeps the engine builder shape identical to
/// the slice-01/02 builders.
const WF_BUDGET_TARGET: &str = "127.0.0.1:9021";

/// The detail the always-transient reference workflow's `ctx.run_retryable`
/// step signals via `RetryableStepError`. A fixed, deterministic string so the
/// run replays bit-identically across seeds. NOTE: this is the STEP's
/// transient signal — the engine absorbs it and never surfaces it on the
/// terminal; the minted `BudgetExhausted` terminal carries the engine's own
/// detail.
const WF_BUDGET_TRANSIENT_DETAIL: &str = "transient: provision call failed (step 04-02 D4)";

/// A reference workflow whose `ctx.run_retryable` STEP always fails
/// transiently: each drive bumps a shared `AtomicUsize` (so the evaluator can
/// count how many times the engine re-drove the body) and the step closure
/// returns `Err(RetryableStepError)` — the engine-absorbed TRANSIENT channel
/// (ADR-0065 §4), NEVER a terminal the body authors. The body's
/// `Result<(), TerminalError>` return type carries NO failure. The engine
/// re-drives up to `WORKFLOW_RETRY_BUDGET`, then mints `BudgetExhausted`.
///
/// Mirrors NEW-5's `AlwaysTransientWorkflow` — the DST invariant drives the
/// identical fixture shape the example-based acceptance does, so both tiers
/// pin the same engine-owned-retry behaviour.
struct AlwaysTransientFailure {
    /// Bumped once per drive (inside the `ctx.run_retryable` step). The engine
    /// re-drives the whole body from the journal on each retry; the transient
    /// step is NOT journaled, so it re-fires every drive. The total count
    /// across all re-drives is the observable proof the engine — not the body
    /// — owned the retry loop.
    attempts: Arc<std::sync::atomic::AtomicUsize>,
}

impl AlwaysTransientFailure {
    /// The workflow name this fixture registers under. Kebab-case, matching
    /// the `WorkflowName` grammar.
    const WORKFLOW_NAME: &'static str = "always-transient-failure";

    /// The concrete [`WorkflowStart`] this fixture corresponds to — a unit
    /// `Input` (the 1-byte CBOR encoding of `()`).
    fn spec() -> WorkflowStart {
        let mut input: Vec<u8> = Vec::new();
        ciborium::into_writer(&(), &mut input)
            .unwrap_or_else(|_| unreachable!("CBOR-encoding the unit type is total"));
        WorkflowStart {
            name: WorkflowName::new(Self::WORKFLOW_NAME)
                .unwrap_or_else(|_| unreachable!("WORKFLOW_NAME is a valid kebab constant")),
            input,
        }
    }
}

#[async_trait::async_trait]
impl Workflow for AlwaysTransientFailure {
    type Output = ();
    type Input = ();

    async fn run(&self, ctx: &WorkflowCtx, _input: ()) -> Result<(), TerminalError> {
        // The transient failure is signalled at the STEP level: the
        // `ctx.run_retryable` closure bumps the drive counter and returns
        // `Err(RetryableStepError)`. The engine ABSORBS it (the step is not
        // journaled; the ctx records a TransientStep; `run_erased` surfaces
        // WorkflowDriveError::Transient) and re-drives. The body authors NO
        // terminal — a step transient cannot become a `TerminalError`; the
        // engine mints `BudgetExhausted` once the budget is consumed (D4).
        let attempts = Arc::clone(&self.attempts);
        let _step: Result<(), _> = ctx
            .run_retryable("provision", async move {
                attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err::<(), RetryableStepError>(RetryableStepError::new(WF_BUDGET_TRANSIENT_DETAIL))
            })
            .await;
        // Unreachable in practice — the engine re-drives off the recorded ctx
        // transient before the body's return value is consulted. Returning Ok
        // keeps the body's terminal channel empty, proving the body authored
        // no failure (the OUTCOME the invariant asserts).
        Ok(())
    }
}

/// Count `RetryAttempted` commands in a loaded run — the engine's durable
/// retry bookkeeping (one per re-drive, the recomputed attempt INPUTS per
/// `development.md` § "Persist inputs, not derived state").
fn retry_attempted_count(run: &[LoadedEntry]) -> usize {
    run.iter()
        .filter(|e| matches!(e, LoadedEntry::Command(JournalCommand::RetryAttempted { .. })))
        .count()
}

/// Spawn a concurrent ticker that advances `clock` past each backoff window
/// until `stop` is set, so the engine's `clock.sleep(backoff)` re-drive parks
/// release under `SimClock`. The harness — never the SUT — drives logical
/// time (`.claude/rules/testing.md` § "Tier 1 — Deterministic Simulation
/// Testing"); the production engine parks on the injected `Clock` with no
/// DST-only branch (`development.md` § "Production code is not shaped by
/// simulation"). Mirrors NEW-5's `spawn_clock_ticker`.
fn spawn_budget_clock_ticker(
    clock: Arc<SimClock>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while !stop.load(std::sync::atomic::Ordering::SeqCst) {
            // Advance well past the largest backoff so every parked re-drive
            // wakes promptly; `yield_now` hands control to the engine task
            // between ticks on the single-threaded runtime.
            clock.tick(Duration::from_secs(1));
            tokio::task::yield_now().await;
        }
    })
}

/// Evaluate `WorkflowBudgetExhaustionMintsTerminal` (ADR-0065 §D4).
///
/// Drives an `AlwaysTransientFailure` workflow (a `ctx.run_retryable` step
/// that returns `Err(RetryableStepError)` on every drive) through the real
/// `WorkflowEngine` + `SimJournalStore`, advancing `SimClock` past each
/// backoff window via a concurrent ticker so the parked re-drives fire, then
/// asserts:
///   1. the engine re-drove the body up to `WORKFLOW_RETRY_BUDGET` — observed
///      as `attempts == budget + 1` (the initial drive + `budget` re-drives,
///      the `exit_observer` "1 initial + N retries" precedent) AND
///      `RetryAttempted` count == `budget` in the durable journal (AC2);
///   2. the engine MINTED `WorkflowStatus::Failed { terminal }` with
///      `terminal.kind() == BudgetExhausted` (AC2);
///   3. the body NEVER authored a failure of its own — the engine-minted
///      `BudgetExhausted` kind (a kind only the engine produces; the step
///      transient is absorbed and never surfaces on the terminal) IS the
///      observable proof of the engine-owns-retry / body-owns-only-terminal
///      split (AC3 — D4).
///
/// Asserts the OUTCOME (engine-minted `BudgetExhausted`), not the step
/// transient channel, so the invariant stays green regardless of how the
/// transient is signalled.
#[must_use]
pub async fn evaluate_workflow_budget_exhaustion_mints_terminal(seed: u64) -> InvariantResult {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let name = "workflow-budget-exhaustion-mints-terminal";
    // The seed is echoed by the harness's RunReport on failure; embedding it
    // in the cause string makes a `cargo dst --seed <N>` reproduction
    // self-describing from the failure line alone.
    let fail = |cause: String| {
        result(name, InvariantStatus::Fail, "host-0", Some(format!("[seed={seed}] {cause}")))
    };

    let spec = AlwaysTransientFailure::spec();
    let correlation = CorrelationKey::derive(
        "wf-budget-exhaustion-0001",
        &ContentHash::of(AlwaysTransientFailure::WORKFLOW_NAME.as_bytes()),
        "start-workflow",
    );
    let workflow_id = WorkflowId::new("wf-budget-exhaustion-0001")
        .unwrap_or_else(|_| unreachable!("wf-budget-exhaustion-0001 is a valid instance id"));

    let journal: Arc<dyn JournalStore> = Arc::new(SimJournalStore::new());
    let obs: Arc<dyn ObservationStore> = Arc::new(SimObservationStore::single_peer(
        NodeId::new("local").unwrap_or_else(|_| unreachable!("local is a valid node id")),
        0,
    ));

    // Build the engine over the SHARED journal + obs, with the attempts
    // counter wired into the registered fixture so the evaluator can count
    // how many times the engine re-drove the body.
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_factory = Arc::clone(&attempts);

    let target: SocketAddr = WF_BUDGET_TARGET
        .parse()
        .unwrap_or_else(|_| unreachable!("WF_BUDGET_TARGET is a valid socket addr"));
    let sim_transport = SimTransport::new();
    let _inbox = sim_transport
        .bind_inbox(target)
        .await
        .unwrap_or_else(|_| unreachable!("SimTransport::bind_inbox is total"));

    let sim_clock = Arc::new(SimClock::new());
    let transport: Arc<dyn TransportTrait> = Arc::new(sim_transport);
    let clock: Arc<dyn Clock> = Arc::clone(&sim_clock) as Arc<dyn Clock>;
    let entropy: Arc<dyn Entropy> = Arc::new(SimEntropy::new(seed));

    let mut registry = WorkflowRegistry::new();
    registry.register(AlwaysTransientFailure::spec().name, move || AlwaysTransientFailure {
        attempts: Arc::clone(&attempts_for_factory),
    });

    let engine =
        WorkflowEngine::new(journal.clone(), clock, transport, entropy, registry, obs.clone());

    // Drive the SimClock concurrently so the engine's backoff parks release —
    // the harness driving logical time, the canonical DST shape.
    let stop = Arc::new(AtomicBool::new(false));
    let ticker = spawn_budget_clock_ticker(Arc::clone(&sim_clock), Arc::clone(&stop));

    let _ = engine.start(&spec, &correlation, &workflow_id).await;
    engine.join_all().await;
    stop.store(true, Ordering::SeqCst);
    let _ = ticker.await;

    // AC2 (re-drove to budget — drive count): the body ran `budget + 1` times,
    // the INITIAL drive plus `WORKFLOW_RETRY_BUDGET` re-drives. The budget
    // bounds the number of RE-DRIVES (the durable `RetryAttempted` count
    // below), not the total drive count; the (budget+1)-th drive is the one
    // that observes the exhausted budget and the engine mints `BudgetExhausted`.
    let drives = attempts.load(Ordering::SeqCst);
    let expected_drives = WORKFLOW_RETRY_BUDGET as usize + 1;
    if drives != expected_drives {
        return fail(format!(
            "body ran {drives} times; expected the initial drive + WORKFLOW_RETRY_BUDGET \
             ({WORKFLOW_RETRY_BUDGET}) re-drives = {expected_drives}"
        ));
    }

    // AC2 (re-drove to budget — durable SSOT): the journal carries
    // `budget`-many `RetryAttempted` commands — the recomputed attempt INPUTS
    // (D4). This is the durable record of exactly how many re-drives the
    // engine performed before minting `BudgetExhausted`.
    let run = journal.load_journal(&workflow_id).await.unwrap_or_default();
    let retries = retry_attempted_count(&run);
    if retries != WORKFLOW_RETRY_BUDGET as usize {
        return fail(format!(
            "journal carries {retries} RetryAttempted commands; expected WORKFLOW_RETRY_BUDGET \
             ({WORKFLOW_RETRY_BUDGET}) — one per re-drive: {run:?}"
        ));
    }

    // AC2 + AC3 (engine-minted terminal): the observable `WorkflowTerminal`
    // row carries `Failed { terminal }` with `kind() == BudgetExhausted`.
    let terminals = match obs.workflow_terminal_rows().await {
        Ok(rows) => rows,
        Err(err) => return fail(format!("workflow_terminal_rows read failed: {err}")),
    };
    let Some((_, status)) = terminals.iter().find(|(corr, _)| *corr == correlation) else {
        return fail(
            "engine wrote no WorkflowTerminal observation row for the budget-exhaustion run"
                .to_owned(),
        );
    };

    let WorkflowStatus::Failed { terminal } = status else {
        return fail(format!(
            "expected WorkflowStatus::Failed on budget exhaustion, got {status:?}"
        ));
    };

    // AC3 — the body authored NO failure: its `ctx.run_retryable` step failed
    // transiently on every drive, which the engine absorbed and re-drove.
    // `BudgetExhausted` is a kind ONLY the engine mints (the body cannot author
    // it — it has no access to the budget, and a step transient is not a
    // `TerminalError`); observing it on the terminal IS the proof the terminal
    // is engine-minted, not body-authored (D4). Asserting the OUTCOME (the
    // minted kind) keeps the invariant robust regardless of how the transient
    // is signalled.
    if terminal.kind() != TerminalErrorKind::BudgetExhausted {
        return fail(format!(
            "expected engine-minted TerminalErrorKind::BudgetExhausted (the body authored no \
             terminal — its step failed transiently, which the engine absorbed), got {:?}",
            terminal.kind()
        ));
    }

    result(name, InvariantStatus::Pass, "host-0", None)
}

// ---------------------------------------------------------------------------
// EntropyDeterminismUnderReseed
// ---------------------------------------------------------------------------

/// Evaluate the entropy determinism invariant for a single seed —
/// two `SimEntropy` instances seeded with `seed` produce identical
/// draw sequences.
#[must_use]
pub fn evaluate_entropy_determinism(seed: u64) -> InvariantResult {
    let a = SimEntropy::new(seed);
    let b = SimEntropy::new(seed);
    evaluate_entropy_determinism_against(&a, &b)
}

/// Evaluate the entropy determinism invariant against two instances.
///
/// Used by the planted-failure unit test that passes differently-seeded
/// entropies and asserts the evaluator catches the disagreement.
#[must_use]
pub fn evaluate_entropy_determinism_against(a: &SimEntropy, b: &SimEntropy) -> InvariantResult {
    /// Number of draws compared across the two entropy instances.
    /// Larger than a handful so a mutation that returns a constant
    /// first draw cannot hide a full-stream divergence.
    const DRAWS: usize = 1_024;

    let name = "entropy-determinism-under-reseed";
    for i in 0..DRAWS {
        let x = a.u64();
        let y = b.u64();
        if x != y {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("SimEntropy diverges at draw {i}: {x:#x} vs {y:#x}")),
            );
        }
    }
    result(name, InvariantStatus::Pass, "host-0", None)
}

// ---------------------------------------------------------------------------
// AtLeastOneReconcilerRegistered (step 04-05)
// ---------------------------------------------------------------------------

/// Evaluate `AtLeastOneReconcilerRegistered` — the registry is never
/// empty after boot.
///
/// Per whitepaper §18 and ADR-0013 §2, a control-plane boot with zero
/// registered reconcilers is a silent-failure shape: the cluster sees
/// no convergence pressure and the operator sees no error. Phase 1
/// registers `noop-heartbeat` as proof-of-life; this invariant catches
/// any future regression that skips registration.
///
/// The harness passes the count of registered reconcilers it composed;
/// the evaluator asserts the count is non-zero. `count` rather than a
/// trait-object dependency on `overdrive-control-plane` keeps the sim
/// crate a leaf adapter.
#[must_use]
pub fn evaluate_at_least_one_reconciler_registered(registered_count: usize) -> InvariantResult {
    let name = "at-least-one-reconciler-registered";
    if registered_count >= 1 {
        result(name, InvariantStatus::Pass, "host-0", None)
    } else {
        result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some("reconciler registry is empty after boot".to_owned()),
        )
    }
}

// ---------------------------------------------------------------------------
// DuplicateEvaluationsCollapse (step 04-05)
// ---------------------------------------------------------------------------

/// Observable broker counters the `DuplicateEvaluationsCollapse`
/// evaluator inspects. Re-exported from `overdrive_core::eval_broker`.
pub type BrokerCountersSnapshot = overdrive_core::eval_broker::BrokerCounters;

/// Evaluate `DuplicateEvaluationsCollapse`.
///
/// N (≥3) concurrent evaluations at the same
/// `(ReconcilerName, TargetResource)` key collapse to exactly one
/// dispatched invocation and `N - 1` cancellations, per ADR-0013 §8
/// storm-proofing.
///
/// The harness is responsible for driving the submit-N-at-same-key +
/// drain sequence; the evaluator inspects the resulting counter
/// snapshot. Passing requires `dispatched == 1`, `cancelled == n - 1`,
/// and `queued == 0` (drain completed).
#[must_use]
pub fn evaluate_duplicate_evaluations_collapse(
    n_submitted: u64,
    counters: BrokerCountersSnapshot,
) -> InvariantResult {
    let name = "duplicate-evaluations-collapse";

    if n_submitted < 3 {
        return result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some(format!(
                "harness submitted {n_submitted} evaluations — invariant requires at least 3",
            )),
        );
    }

    let expected_cancelled = n_submitted - 1;
    if counters.dispatched == 1 && counters.cancelled == expected_cancelled && counters.queued == 0
    {
        result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
    } else {
        result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some(format!(
                "expected dispatched=1 cancelled={expected_cancelled} queued=0 after {n_submitted} same-key submits; got {counters:?}",
            )),
        )
    }
}

// ---------------------------------------------------------------------------
// BrokerDrainOrderIsDeterministic (step 01-05)
// ---------------------------------------------------------------------------

/// Observable per-pass drain order the
/// `BrokerDrainOrderIsDeterministic` evaluator inspects.
///
/// Sibling to [`BrokerCountersSnapshot`] — counters proves the LWW
/// key-collapse invariant, this snapshot proves drain-order
/// determinism. Both coexist; neither replaces the other.
///
/// The harness captures one of these from a FIRST drain pass and a
/// SECOND drain pass (each drain replays identical submit semantics)
/// and the evaluator asserts the two `dispatched_order` vecs are
/// element-equal in the same positions. A divergence at any position
/// means the broker's drain order depends on something other than the
/// submit sequence — `HashSet` iteration order, allocator placement,
/// thread scheduling — and the invariant fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerDrainOrderSnapshot {
    /// The ordered sequence of `(ReconcilerName, TargetResource)` keys
    /// the broker dispatched during a single drain pass.
    pub dispatched_order: Vec<(
        overdrive_core::reconcilers::ReconcilerName,
        overdrive_core::reconcilers::TargetResource,
    )>,
}

/// Evaluate `BrokerDrainOrderIsDeterministic`.
///
/// Two drain passes against identical submit sequences must produce
/// element-equal `dispatched_order` vecs at every position. On
/// mismatch, the failure cause names the first divergent position by
/// index — a structured signal mutation testing can target precisely.
///
/// The harness is responsible for driving two copies of the same
/// submit-and-drain sequence and capturing both
/// `BrokerDrainOrderSnapshot`s; this evaluator inspects only the
/// snapshots.
#[must_use]
pub fn evaluate_broker_drain_order_is_deterministic(
    a: &BrokerDrainOrderSnapshot,
    b: &BrokerDrainOrderSnapshot,
) -> InvariantResult {
    let name = "broker-drain-order-is-deterministic";

    // Find the first divergent position via zip().enumerate().find().
    // Length mismatch is also a divergence: the shorter vec ends
    // first, so a missing trailing entry shows up as a position-equal
    // length comparison after the zip exhausts.
    let first_divergence = a
        .dispatched_order
        .iter()
        .zip(b.dispatched_order.iter())
        .enumerate()
        .find(|(_, (lhs, rhs))| lhs != rhs)
        .map(|(idx, _)| idx);

    if first_divergence.is_none() && a.dispatched_order.len() == b.dispatched_order.len() {
        result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
    } else {
        let position = first_divergence
            .unwrap_or_else(|| a.dispatched_order.len().min(b.dispatched_order.len()));
        result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some(format!(
                "drain order diverged at position {position}: \
                 first={:?} second={:?}",
                a.dispatched_order, b.dispatched_order,
            )),
        )
    }
}

// ---------------------------------------------------------------------------
// DispatchRoutingIsNameRestricted (fix-dst-dispatch-routing-invariant 01-01)
// ---------------------------------------------------------------------------

/// Evaluation type used by the `DispatchRoutingIsNameRestricted`
/// evaluator. Re-exported from `overdrive_core::eval_broker`.
pub use overdrive_core::eval_broker::Evaluation;

/// Snapshot of dispatcher invocations during one tick.
///
/// Each entry is one `(reconciler, target)` tuple from a single
/// `run_convergence_tick` invocation. Sibling to
/// [`BrokerCountersSnapshot`] (broker-side entry collapse) and
/// [`BrokerDrainOrderSnapshot`] (broker-side drain order); all three
/// coexist and none subsumes another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchRecord {
    /// Each `(reconciler, target)` the dispatcher invoked. Order
    /// reflects the dispatcher's call order, but the invariant is
    /// permutation-invariant on the target axis (one entry per drained
    /// eval, per the §8 entry-collapse contract).
    pub dispatched: Vec<(
        overdrive_core::reconcilers::ReconcilerName,
        overdrive_core::reconcilers::TargetResource,
    )>,
}

/// Evaluate `DispatchRoutingIsNameRestricted`.
///
/// For every drained `Evaluation { reconciler: R, target: T }` the
/// harness submitted, the dispatch record MUST contain exactly one
/// entry `(R, T)` and zero entries `(R', T)` for any `R' != R`. This
/// pins the §8 storm-proofing dispatch-routing contract end-to-end:
/// the DST-tier peer of the unit/acceptance pin at
/// `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs::eval_dispatch_runs_only_the_named_reconciler`
/// (commit `e6f5e5e`).
///
/// Four branches in this exact order:
///
/// 1. **Vacuous-pass** on empty input — the invariant is "for every
///    drained eval ..." and ∀∅ holds trivially. Without this the
///    empty-input case would misreport as a fail under the cardinality
///    branch.
/// 2. **Cardinality** — `record.dispatched.len() == submitted.len()`.
///    A surplus entry is a fan-out regression; a deficit is a missed
///    dispatch.
/// 3. **Per-eval routing** — for every submitted `(R, T)`, exactly one
///    matching entry in `record.dispatched`. Catches the "named
///    reconciler not dispatched" shape under clean cardinality.
/// 4. **Smoking-gun** — any dispatch entry naming a reconciler outside
///    the submitted set is a fan-out smoking gun. Catches the precise
///    bug shape the precursor fix closed: a `run_convergence_tick`
///    that iterates the registry rather than looking up by name.
#[must_use]
pub fn evaluate_dispatch_routing_is_name_restricted(
    submitted: &[Evaluation],
    record: &DispatchRecord,
) -> InvariantResult {
    let name = "dispatch-routing-is-name-restricted";

    // (a) Vacuous-pass on empty input.
    if submitted.is_empty() {
        return result(name, InvariantStatus::Pass, CLUSTER_HOST, None);
    }

    // (b) Cardinality check.
    if record.dispatched.len() != submitted.len() {
        return result(
            name,
            InvariantStatus::Fail,
            CLUSTER_HOST,
            Some(format!(
                "expected {expected} dispatch entries (one per drained eval), got {got}: \
                 dispatched={dispatched:?}",
                expected = submitted.len(),
                got = record.dispatched.len(),
                dispatched = record.dispatched,
            )),
        );
    }

    // (c) Per-eval routing — every submitted (R, T) must appear exactly
    //     once in the dispatch record. A count of zero means the named
    //     reconciler was not dispatched; a count > 1 means it was
    //     dispatched more than once.
    for eval in submitted {
        let key = (eval.reconciler.clone(), eval.target.clone());
        let matches = record.dispatched.iter().filter(|d| **d == key).count();
        if matches != 1 {
            return result(
                name,
                InvariantStatus::Fail,
                CLUSTER_HOST,
                Some(format!(
                    "expected exactly one dispatch of ({reconciler}, {target}) — \
                     the named reconciler — got {matches} entries: dispatched={dispatched:?}",
                    reconciler = eval.reconciler,
                    target = eval.target,
                    matches = matches,
                    dispatched = record.dispatched,
                )),
            );
        }
    }

    // (d) Smoking-gun — any dispatched entry naming a reconciler NOT in
    //     the submitted set is a fan-out regression.
    let submitted_names: std::collections::BTreeSet<&overdrive_core::reconcilers::ReconcilerName> =
        submitted.iter().map(|e| &e.reconciler).collect();
    for (r, t) in &record.dispatched {
        if !submitted_names.contains(r) {
            return result(
                name,
                InvariantStatus::Fail,
                CLUSTER_HOST,
                Some(format!(
                    "dispatcher invoked unsubmitted reconciler {r} against {t} — fan-out regression",
                )),
            );
        }
    }

    result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
}

// ---------------------------------------------------------------------------
// ReconcilerIsPure (step 04-05)
// ---------------------------------------------------------------------------

/// Evaluate `ReconcilerIsPure` — twin invocation of `reconciler.reconcile`
/// with identical `(desired, actual, view, tick)` inputs produces
/// bit-identical `(Vec<Action>, NextView)` tuples.
///
/// This is the runtime witness of the ADR-0013 §2 purity contract. A
/// reconciler that smuggles non-determinism (wall-clock read,
/// `rand::thread_rng`, internal `RefCell` counter, ...) fails here.
/// Phase 1 runs this against the `noop-heartbeat` reconciler, which
/// always returns `vec![Action::Noop]` — a deterministic baseline that
/// proves the machinery is live.
///
/// # Time injection
///
/// The `TickContext` passed to `reconcile` pulls its `now` from the
/// caller-supplied `SimClock`, NOT from `std::time::Instant::now()`.
/// Under DST, `SimClock::now()` is seed-deterministic — two harness
/// runs at the same seed see the same `now`, and the twin invocation
/// within a single run sees one `now` shared across both calls (the
/// same `TickContext` reference is passed to each). This preserves
/// the ADR-0013 §2c "time is input state, injected once per tick"
/// contract even at the sim-layer callsite.
#[must_use]
pub fn evaluate_reconciler_is_pure(
    reconciler: &AnyReconciler,
    clock: &SimClock,
) -> InvariantResult {
    let name = "reconciler-is-pure";

    // Twin invocation with identical inputs per ADR-0013 §2 / §2c. ONE
    // `TickContext` is constructed and passed to BOTH calls so time is
    // a shared input, not a per-call side channel. The full §18 purity
    // semantics (pre-hydrated view + next-view tuple return) are
    // exercised here — `(actions, next_view)` are asserted as paired
    // but separate bit-identical comparisons so a mutation that drops
    // either side is caught.
    //
    // Per ADR-0021 (step 02-01), `desired`/`actual` are now typed
    // `&AnyState` rather than the prior `&State, &State` placeholder
    // pair. Phase 1 reconcilers (`NoopHeartbeat`) use `AnyState::Unit`
    // because their `Reconciler::State = ()`; the `WorkloadLifecycle`
    // reconciler's `AnyState::WorkloadLifecycle(...)` arm becomes reachable
    // in step 02-03 when the runtime tick loop ships.
    let desired = AnyState::Unit;
    let actual = AnyState::Unit;
    let view = AnyReconcilerView::Unit;
    let tick = build_tick_context(clock);

    let (actions_a, next_view_a) = reconciler.reconcile(&desired, &actual, &view, &tick);
    let (actions_b, next_view_b) = reconciler.reconcile(&desired, &actual, &view, &tick);

    if actions_a == actions_b && next_view_a == next_view_b {
        result(name, InvariantStatus::Pass, "host-0", None)
    } else {
        result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some(format!(
                "reconciler {} diverged under twin invocation: \
                 first=(actions={actions_a:?}, next_view={next_view_a:?}) \
                 second=(actions={actions_b:?}, next_view={next_view_b:?})",
                reconciler.name(),
            )),
        )
    }
}

/// Construct the `TickContext` snapshot handed to both twin
/// invocations inside `evaluate_reconciler_is_pure`.
///
/// Extracted from the evaluator body so the `now + BUDGET` arithmetic
/// can be unit-tested. With the `TickContext` construction inlined,
/// the `+` mutation survived — the Phase 1 reconciler fixture
/// (`NoopHeartbeat`) ignores `tick.deadline` entirely, so a
/// deadline-in-the-past produced no observable divergence. The
/// helper form gives mutation testing a direct target: a unit test
/// asserts `tick.deadline > tick.now` and `+ -> -` flips the sign
/// on that difference.
///
/// The `tick` counter stays zero (the evaluator runs once per harness
/// pass, not inside a real reconcile loop) and the budget is a
/// 1-second literal matching the 04-07 test-side `TickContext`
/// construction — no injected production budget exists yet per ADR-0013.
#[inline]
fn build_tick_context(clock: &SimClock) -> TickContext {
    /// Monotonic tick counter — the evaluator runs once per harness
    /// pass (not inside a real reconcile loop), so a fixed zero is
    /// the right shape. The field exists to give reconcilers a
    /// deterministic tie-breaker that does not depend on wall-clock
    /// granularity; the harness's single-shot nature means there is
    /// no per-call progression to model.
    const TICK: u64 = 0;
    /// Per-evaluation reconcile budget. No injected production
    /// budget yet (§14 right-sizing will provide one); a 1-second
    /// literal matches the 04-07 test-side `TickContext`
    /// construction.
    const BUDGET: Duration = Duration::from_secs(1);

    let now = clock.now();
    let now_unix = UnixInstant::from_clock(clock);
    TickContext { now, now_unix, tick: TICK, deadline: now + BUDGET }
}

// ---------------------------------------------------------------------------
// phase-1-first-workload — slice 3 (US-03) — convergence invariants
// ---------------------------------------------------------------------------

/// Evaluate `JobScheduledAfterSubmission` (eventually).
///
/// Per US-03 AC: for every submitted job in `submitted_jobs`, an
/// `AllocStatusRow{state: Running}` referencing that job MUST exist in
/// `alloc_status` after the convergence-loop budget elapsed. The harness
/// drives the runtime tick loop forward N ticks and then snapshots the
/// `ObservationStore`'s `alloc_status` table; that snapshot is fed to
/// this evaluator.
///
/// **Pure verdict** — `submitted_jobs` is the desired set;
/// `alloc_status` is the actual observation. The evaluator counts pass
/// iff every submitted job has at least one Running alloc in the
/// snapshot. Pure, no I/O — the harness owns the tick loop.
#[must_use]
pub fn evaluate_workload_scheduled_after_submission(
    submitted_jobs: &[WorkloadId],
    alloc_status: &[AllocStatusRow],
) -> InvariantResult {
    let name = "job-scheduled-after-submission";
    // Vacuous-pass when no jobs were submitted — the invariant is
    // "for every submitted job ..." and ∀ ∅ holds trivially. Without
    // this the empty-input case would surface as a "0 jobs running"
    // false positive on the next mutation of the `submitted_jobs.len()`
    // comparison.
    if submitted_jobs.is_empty() {
        return result(name, InvariantStatus::Pass, CLUSTER_HOST, None);
    }
    for workload_id in submitted_jobs {
        let has_running = alloc_status
            .iter()
            .any(|row| &row.workload_id == workload_id && row.state == AllocState::Running);
        if !has_running {
            return result(
                name,
                InvariantStatus::Fail,
                CLUSTER_HOST,
                Some(format!("submitted job {workload_id} has no Running alloc within budget")),
            );
        }
    }
    result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
}

/// Evaluate `DesiredReplicaCountConverges` (eventually).
///
/// Per US-03 AC: `count(state == Running for job_j) == replicas_j` for
/// every submitted job. Phase 1 jobs are 1-replica so this is a
/// vacuous-pass at N=1 — but the evaluator still walks the rows to
/// catch the leak-across-jobs case where one job's Running row is
/// double-counted against another.
///
/// `desired_replicas` carries the per-job replica count; `alloc_status`
/// is the observation snapshot.
#[must_use]
pub fn evaluate_desired_replica_count_converges(
    desired_replicas: &[(WorkloadId, u32)],
    alloc_status: &[AllocStatusRow],
) -> InvariantResult {
    let name = "desired-replica-count-converges";
    for (workload_id, want) in desired_replicas {
        let running_count = u32::try_from(
            alloc_status
                .iter()
                .filter(|row| &row.workload_id == workload_id && row.state == AllocState::Running)
                .count(),
        )
        .unwrap_or(u32::MAX);
        if running_count != *want {
            return result(
                name,
                InvariantStatus::Fail,
                CLUSTER_HOST,
                Some(format!("job {workload_id}: want {want} Running, observed {running_count}")),
            );
        }
    }
    result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
}

/// Evaluate `NoDoubleScheduling` (always).
///
/// Per US-03 AC: every `alloc_id` in the snapshot agrees on a single
/// `node_id`. Two rows for the same `alloc_id` pinned to different
/// nodes is a double-scheduling violation. Operates on a single
/// snapshot; the "always" semantics come from the harness invoking
/// this evaluator on every tick rather than just at the end.
///
/// Implementation note: a `BTreeMap<AllocationId, NodeId>` accumulates
/// the first observed pin and rejects subsequent rows whose node
/// differs.
#[must_use]
pub fn evaluate_no_double_scheduling(alloc_status: &[AllocStatusRow]) -> InvariantResult {
    let name = "no-double-scheduling";
    let mut pinned: std::collections::BTreeMap<overdrive_core::id::AllocationId, NodeId> =
        std::collections::BTreeMap::new();
    for row in alloc_status {
        if let Some(prior) = pinned.get(&row.alloc_id) {
            if prior != &row.node_id {
                return result(
                    name,
                    InvariantStatus::Fail,
                    CLUSTER_HOST,
                    Some(format!(
                        "alloc {} pinned to two nodes: {prior} and {}",
                        row.alloc_id, row.node_id,
                    )),
                );
            }
        } else {
            pinned.insert(row.alloc_id.clone(), row.node_id.clone());
        }
    }
    result(name, InvariantStatus::Pass, CLUSTER_HOST, None)
}

// ---------------------------------------------------------------------------
// reconciler-memory-redb step 01-07 — ViewStore DST invariants
// ---------------------------------------------------------------------------

/// Verdict shape for the per-case roundtrip comparison.
///
/// Extracted as a typed enum so unit tests can exercise every branch
/// without spinning up a real `SimViewStore`. The match arm shape
/// (`Match` / `Mismatch` / `Absent`) mirrors the three observable
/// outcomes of decoding a roundtripped value: identical, different,
/// or missing.
#[derive(Debug, PartialEq, Eq)]
pub enum RoundtripVerdict {
    /// The decoded value is byte-equal to the original.
    Match,
    /// The decoded value is present but differs from the original.
    Mismatch,
    /// No decoded value was present at the expected key.
    Absent,
}

/// Pure verdict for a single roundtrip case.
///
/// Compares `decoded` against `expected` using `PartialEq`. Mutation-
/// testable: a flip of `==` to `!=` on the underlying comparison
/// surfaces immediately because the unit tests below exercise both
/// `Match` and `Mismatch` branches.
#[must_use]
pub fn roundtrip_verdict<V: PartialEq>(decoded: Option<&V>, expected: &V) -> RoundtripVerdict {
    match decoded {
        Some(v) if v == expected => RoundtripVerdict::Match,
        Some(_) => RoundtripVerdict::Mismatch,
        None => RoundtripVerdict::Absent,
    }
}

/// Verdict shape for the `WriteThroughOrdering` in-memory check.
///
/// Three observable outcomes after a failed `write_through`:
/// `OrderingHeld` (in-memory still carries the original — the contract
/// holds), `Advanced` (in-memory carries the would-be `next_view` — the
/// contract is violated), `UnexpectedView` (in-memory carries something
/// other than original or `next_view` — corrupt state), or `Absent` (no
/// entry — register did not bulk-load).
#[derive(Debug, PartialEq, Eq)]
pub enum OrderingVerdict {
    /// In-memory still carries the original — fsync-then-memory ordering held.
    OrderingHeld,
    /// In-memory advanced to `next_view` despite fsync failure — violation.
    Advanced,
    /// In-memory carries an unexpected view — corrupt state.
    UnexpectedView,
    /// No in-memory entry for the target — register did not bulk-load.
    Absent,
}

/// Pure verdict for the in-memory ordering check.
///
/// Compares `loaded` against the `original` (pre-injection value) and
/// the `would_be_next_view` (post-injection candidate that should NOT
/// have landed). Mutation-testable: each branch fires under a distinct
/// fixture, so flipping any guard surfaces immediately.
#[must_use]
pub fn ordering_verdict<V: PartialEq>(
    loaded: Option<&V>,
    original: &V,
    would_be_next_view: &V,
) -> OrderingVerdict {
    match loaded {
        Some(v) if v == original => OrderingVerdict::OrderingHeld,
        Some(v) if v == would_be_next_view => OrderingVerdict::Advanced,
        Some(_) => OrderingVerdict::UnexpectedView,
        None => OrderingVerdict::Absent,
    }
}

/// Evaluate `ViewStoreRoundtripIsLossless`.
///
/// proptest-backed: for arbitrary `View` values, `write_through` then
/// `bulk_load` returns byte-equal results. Covers `WorkloadLifecycleView`
/// (the only meaningful production View today) and `()` (the unit-View
/// case used by `NoopHeartbeat`). Catches CBOR encode/decode regressions,
/// ciborium-version skew, and serde-derive oversights per ADR-0035 §6.
///
/// Each generated case constructs a fresh `SimViewStore` so fixtures
/// cannot leak across cases. The proptest case count is bounded by the
/// per-invariant evaluation budget — large enough to exercise the
/// generator's variance without blowing the harness wall-clock.
#[allow(clippy::too_many_lines, clippy::items_after_statements)]
// Justification: the body is sequential setup-then-loop; splitting it
// into helpers would force passing the `name`/`rng`/store handles
// through extra arguments without making the flow clearer. The
// `CASES` const is documented inline with its semantic meaning, which
// is the right place for it (per-evaluator tuning knob).
pub async fn evaluate_view_store_roundtrip_is_lossless(seed: u64) -> InvariantResult {
    use overdrive_control_plane::view_store::ViewStoreExt;
    use overdrive_core::id::AllocationId;
    use overdrive_core::reconcilers::{TargetResource, WorkloadLifecycleView};
    use overdrive_core::wall_clock::UnixInstant;
    use rand::{Rng, SeedableRng};

    use crate::adapters::view_store::SimViewStore;

    /// Cases per invariant evaluation. Larger than a handful so a
    /// generator that returns a constant cannot hide a real divergence,
    /// small enough to keep the per-evaluation wall-clock bounded.
    const CASES: usize = 64;

    let name = "view-store-roundtrip-is-lossless";

    // Seed a chacha rng from the harness seed so the generator is
    // bit-deterministic across runs at the same seed (K3 reproducibility).
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);

    // Scope the case loop into a closure so we can early-return on the
    // first divergence with a structured cause.
    // Reconciler names are passed to the ViewStore as
    // `<Concrete as Reconciler>::NAME` consts directly — `&'static str`
    // per the `refactor-reconciler-static-name` RCA — so no
    // `ReconcilerName::new(...)` wrapping is needed at the call site.
    for case_idx in 0..CASES {
        // Generate a WorkloadLifecycleView with random restart_counts and
        // last_failure_seen_at maps. Cardinality 0..4 covers empty,
        // single-entry, and multi-entry shapes.
        let entries: usize = rng.gen_range(0..4);
        let mut view = WorkloadLifecycleView::default();
        for i in 0..entries {
            let raw = format!("alloc-case-{case_idx}-{i}");
            let alloc_id = match AllocationId::new(&raw) {
                Ok(a) => a,
                Err(err) => {
                    return result(
                        name,
                        InvariantStatus::Fail,
                        "host-0",
                        Some(format!("alloc id construction failed: {err}")),
                    );
                }
            };
            let count: u32 = rng.r#gen();
            view.restart_counts.insert(alloc_id.clone(), count);
            // Half the time also stamp a last_failure_seen_at so both
            // map fields exercise the roundtrip path.
            if rng.r#gen::<bool>() {
                let secs: u64 = rng.gen_range(0..u64::from(u32::MAX));
                view.last_failure_seen_at
                    .insert(alloc_id, UnixInstant::from_unix_duration(Duration::from_secs(secs)));
            }
        }

        // Roundtrip WorkloadLifecycleView through a fresh SimViewStore.
        let store = SimViewStore::new();
        let target_raw = format!("job/case-{case_idx}");
        let target = match TargetResource::new(&target_raw) {
            Ok(t) => t,
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!("target construction failed: {err}")),
                );
            }
        };
        if let Err(err) =
            store.write_through(<WorkloadLifecycle as Reconciler>::NAME, &target, &view).await
        {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("case {case_idx}: write_through failed: {err}")),
            );
        }
        let loaded: std::collections::BTreeMap<TargetResource, WorkloadLifecycleView> =
            match store.bulk_load(<WorkloadLifecycle as Reconciler>::NAME).await {
                Ok(m) => m,
                Err(err) => {
                    return result(
                        name,
                        InvariantStatus::Fail,
                        "host-0",
                        Some(format!("case {case_idx}: bulk_load failed: {err}")),
                    );
                }
            };
        match roundtrip_verdict(loaded.get(&target), &view) {
            RoundtripVerdict::Match => {}
            RoundtripVerdict::Mismatch => {
                let decoded = loaded.get(&target);
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!(
                        "case {case_idx}: WorkloadLifecycleView roundtrip diverged — \
                         original={view:?} decoded={decoded:?}",
                    )),
                );
            }
            RoundtripVerdict::Absent => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!(
                        "case {case_idx}: WorkloadLifecycleView absent from bulk_load result"
                    )),
                );
            }
        }
    }

    // Unit-View roundtrip — `()` is the View shape `NoopHeartbeat`
    // uses. Encode/decode of unit must succeed with byte-equal payload.
    let store = SimViewStore::new();
    let target_unit = match TargetResource::new("job/unit-case") {
        Ok(t) => t,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("unit target construction failed: {err}")),
            );
        }
    };
    let unit_value: () = ();
    if let Err(err) =
        store.write_through(<NoopHeartbeat as Reconciler>::NAME, &target_unit, &unit_value).await
    {
        return result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some(format!("unit-view write_through failed: {err}")),
        );
    }
    // `BTreeMap<TargetResource, ()>` — deliberate; `()` IS the
    // unit-View shape under test. clippy::zero_sized_map_values is
    // allowed because the map values are exactly what we are
    // verifying roundtrip cleanly.
    #[allow(clippy::zero_sized_map_values)]
    let unit_loaded: std::collections::BTreeMap<TargetResource, ()> =
        match store.bulk_load(<NoopHeartbeat as Reconciler>::NAME).await {
            Ok(m) => m,
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!("unit-view bulk_load failed: {err}")),
                );
            }
        };
    if unit_loaded.get(&target_unit) != Some(&()) {
        return result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some("unit-view roundtrip diverged — () did not roundtrip via CBOR".to_owned()),
        );
    }

    result(name, InvariantStatus::Pass, "host-0", None)
}

/// Evaluate `BulkLoadIsDeterministic`.
///
/// Pre-populate a `SimViewStore` with a fixed corpus of (target, view)
/// pairs spanning multiple reconcilers and ≥3 targets per reconciler.
/// Call `bulk_load` twice; assert `PartialEq` equality (not just
/// `.len()`). Catches `BTreeMap → HashMap` regressions or any other
/// mutation that would destabilise iteration order.
#[allow(clippy::too_many_lines)]
// Justification: body is straight-line corpus setup + two reads +
// equality verdict; extracting helpers would obscure the
// "load fixture, read twice, compare" intent.
pub async fn evaluate_bulk_load_is_deterministic() -> InvariantResult {
    use overdrive_control_plane::view_store::ViewStoreExt;
    use overdrive_core::id::AllocationId;
    use overdrive_core::reconcilers::{TargetResource, WorkloadLifecycleView};

    use crate::adapters::view_store::SimViewStore;

    let name = "bulk-load-is-deterministic";

    // Reconciler name flows through the ViewStore byte surface as
    // `<WorkloadLifecycle as Reconciler>::NAME` directly per the
    // `refactor-reconciler-static-name` RCA — no `ReconcilerName::new`
    // wrapping needed.

    let store = SimViewStore::new();

    // ≥3 targets per reconciler so iteration order is meaningfully
    // exercised. The order chosen here (frontend, payments, scheduler)
    // is deliberately NOT alphabetical so a `BTreeMap`-backed store
    // that sorts by `Ord` reorders to (frontend, payments, scheduler)
    // anyway — but a hypothetical regression that returned insertion
    // order would surface as (payments, frontend, scheduler) and the
    // PartialEq comparison between two passes would still hold under
    // the regression. We assert PartialEq across two READS of the SAME
    // store, which catches NON-DETERMINISM (different order across
    // reads) rather than ORDERING (a specific order). Both calls must
    // produce identical output.
    let targets_with_views: Vec<(&str, WorkloadLifecycleView)> = vec![
        ("job/payments", {
            let mut v = WorkloadLifecycleView::default();
            let id = match AllocationId::new("alloc-payments-0") {
                Ok(a) => a,
                Err(err) => {
                    return result(
                        name,
                        InvariantStatus::Fail,
                        "host-0",
                        Some(format!("alloc id construction failed: {err}")),
                    );
                }
            };
            v.restart_counts.insert(id, 5);
            v
        }),
        ("job/frontend", {
            let mut v = WorkloadLifecycleView::default();
            let id = match AllocationId::new("alloc-frontend-0") {
                Ok(a) => a,
                Err(err) => {
                    return result(
                        name,
                        InvariantStatus::Fail,
                        "host-0",
                        Some(format!("alloc id construction failed: {err}")),
                    );
                }
            };
            v.restart_counts.insert(id, 2);
            v
        }),
        ("job/scheduler", {
            let mut v = WorkloadLifecycleView::default();
            let id = match AllocationId::new("alloc-scheduler-0") {
                Ok(a) => a,
                Err(err) => {
                    return result(
                        name,
                        InvariantStatus::Fail,
                        "host-0",
                        Some(format!("alloc id construction failed: {err}")),
                    );
                }
            };
            v.restart_counts.insert(id, 0);
            v
        }),
    ];

    for (raw, view) in &targets_with_views {
        let target = match TargetResource::new(raw) {
            Ok(t) => t,
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!("target construction failed: {err}")),
                );
            }
        };
        if let Err(err) =
            store.write_through(<WorkloadLifecycle as Reconciler>::NAME, &target, view).await
        {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("seed write_through failed: {err}")),
            );
        }
    }

    // Two bulk_load calls against the same store. Both must produce
    // PartialEq-equal BTreeMaps — same keys, same values, same order
    // (BTreeMap PartialEq compares element-wise in iteration order).
    let first: std::collections::BTreeMap<TargetResource, WorkloadLifecycleView> =
        match store.bulk_load(<WorkloadLifecycle as Reconciler>::NAME).await {
            Ok(m) => m,
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!("first bulk_load failed: {err}")),
                );
            }
        };
    let second: std::collections::BTreeMap<TargetResource, WorkloadLifecycleView> =
        match store.bulk_load(<WorkloadLifecycle as Reconciler>::NAME).await {
            Ok(m) => m,
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!("second bulk_load failed: {err}")),
                );
            }
        };

    if first != second {
        return result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some(format!(
                "bulk_load returned divergent maps across two calls — \
                 first={first:?} second={second:?}",
            )),
        );
    }

    // Sanity check the corpus actually landed (catches a future
    // mutation that returns an empty map and would PASS the trivial
    // `empty == empty` comparison above).
    if first.len() != targets_with_views.len() {
        return result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some(format!(
                "bulk_load returned {} entries; expected {} (corpus mismatch)",
                first.len(),
                targets_with_views.len(),
            )),
        );
    }

    result(name, InvariantStatus::Pass, "host-0", None)
}

/// Evaluate `WriteThroughOrdering`.
///
/// Drives the load-bearing crash-durability primitive from ADR-0035
/// §5 at the `ViewStore` boundary directly: the runtime's
/// fsync-then-memory ordering rule depends on `write_through` being
/// atomic w.r.t. the underlying storage — when fsync fails,
/// `write_through_bytes` MUST return Err AND a subsequent `bulk_load`
/// MUST still return the pre-injection value (NOT the would-be
/// `next_view`).
///
/// The evaluator:
///
/// 1. Seeds an `original` `WorkloadLifecycleView` via `write_through`.
/// 2. Calls `inject_fsync_failure` on the `SimViewStore`.
/// 3. Attempts a second `write_through` with a divergent `next_view`.
///    The call MUST error.
/// 4. Clears the injection and reads back via `bulk_load`. The loaded
///    value MUST still equal `original`, NOT `next_view`.
///
/// This pins the `SimViewStore` contract the runtime relies on for
/// its fsync-first ordering. The runtime-side end-to-end version of
/// this contract is exercised in
/// `crates/overdrive-control-plane/tests/integration/reconciler_runtime_view_store.rs::runtime_writes_through_before_in_memory_update`
/// — that test takes the dependency on the runtime's test-only
/// accessors; the DST invariant stays at the `SimViewStore` level so
/// it can run in any harness composition without the
/// `integration-tests` feature being globally enabled.
#[allow(clippy::too_many_lines)]
// Justification: every match block is a structured-cause early-return
// with a distinct error message. Splitting into helpers would force
// passing `name` + captured fixture values through extra arguments
// without making the seed→inject→assert flow clearer.
pub async fn evaluate_write_through_ordering() -> InvariantResult {
    use overdrive_control_plane::view_store::ViewStoreExt;
    use overdrive_core::id::AllocationId;
    use overdrive_core::reconcilers::{TargetResource, WorkloadLifecycleView};

    use crate::adapters::view_store::SimViewStore;

    let name = "write-through-ordering";

    let sim = SimViewStore::new();
    // Reconciler name flows through the ViewStore byte surface as
    // `<WorkloadLifecycle as Reconciler>::NAME` directly per the
    // `refactor-reconciler-static-name` RCA — no `ReconcilerName::new`
    // wrapping needed.
    let target = match TargetResource::new("job/payments") {
        Ok(t) => t,
        Err(err) => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!("target construction failed: {err}")),
            );
        }
    };

    // STEP 1 — seed the `original` view via a clean `write_through`.
    let original = {
        let mut v = WorkloadLifecycleView::default();
        let id = match AllocationId::new("alloc-payments-0") {
            Ok(a) => a,
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!("alloc id construction failed: {err}")),
                );
            }
        };
        v.restart_counts.insert(id, 7);
        v
    };
    if let Err(err) =
        sim.write_through(<WorkloadLifecycle as Reconciler>::NAME, &target, &original).await
    {
        return result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some(format!("seed write_through failed: {err}")),
        );
    }

    // STEP 2 — construct a divergent `next_view` and inject fsync
    // failure. The follow-on `write_through` MUST error.
    let next_view = {
        let mut v = original.clone();
        let id = match AllocationId::new("alloc-payments-0") {
            Ok(a) => a,
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!("alloc id construction failed: {err}")),
                );
            }
        };
        v.restart_counts.insert(id, 99);
        v
    };

    sim.inject_fsync_failure();
    let write_result =
        sim.write_through(<WorkloadLifecycle as Reconciler>::NAME, &target, &next_view).await;
    sim.clear_fsync_failure();

    if write_result.is_ok() {
        return result(
            name,
            InvariantStatus::Fail,
            "host-0",
            Some(
                "write_through returned Ok under fsync injection — \
                 the SimViewStore should have surfaced the fsync error"
                    .to_owned(),
            ),
        );
    }

    // STEP 3 — read back via `bulk_load`. The loaded value MUST still
    // equal `original`, NOT `next_view`. This is the load-bearing
    // contract: the runtime's in-memory `BTreeMap` is updated only
    // after `write_through` returns Ok, so the underlying storage
    // MUST roll back any partial write when fsync fails.
    let loaded: std::collections::BTreeMap<TargetResource, WorkloadLifecycleView> =
        match sim.bulk_load(<WorkloadLifecycle as Reconciler>::NAME).await {
            Ok(m) => m,
            Err(err) => {
                return result(
                    name,
                    InvariantStatus::Fail,
                    "host-0",
                    Some(format!("post-injection bulk_load failed: {err}")),
                );
            }
        };
    match ordering_verdict(loaded.get(&target), &original, &next_view) {
        OrderingVerdict::OrderingHeld => {}
        OrderingVerdict::Advanced => {
            let observed = loaded.get(&target);
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!(
                    "storage advanced to next_view despite fsync failure — \
                     ordering violation; observed={observed:?}",
                )),
            );
        }
        OrderingVerdict::UnexpectedView => {
            let observed = loaded.get(&target);
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!(
                    "storage carries unexpected view after failed write — \
                     observed={observed:?} expected={original:?}",
                )),
            );
        }
        OrderingVerdict::Absent => {
            return result(
                name,
                InvariantStatus::Fail,
                "host-0",
                Some(format!(
                    "storage has no entry for target {target} — \
                     seed write_through must have rolled back too",
                )),
            );
        }
    }

    result(name, InvariantStatus::Pass, "host-0", None)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    //! Library-level witnesses for each evaluator. Paired with the
    //! integration tests under `crates/overdrive-sim/tests/invariant_evaluators.rs`
    //! — the integration tests prove the full contract, these tests
    //! kill the low-hanging mutations in this file.

    use super::*;
    use std::str::FromStr;

    fn n(s: &str) -> NodeId {
        NodeId::from_str(s).expect("valid node id")
    }

    #[test]
    fn topology_with_single_leader_passes_and_names_that_host() {
        let hosts = vec![n("host-0"), n("host-1"), n("host-2")];
        let r = evaluate_single_leader_from_topology(&hosts, Some(&n("host-1")));
        assert_eq!(r.status, InvariantStatus::Pass);
        assert_eq!(r.host, "host-1");
    }

    #[test]
    fn topology_with_leader_outside_participants_fails() {
        let hosts = vec![n("host-0"), n("host-1")];
        let r = evaluate_single_leader_from_topology(&hosts, Some(&n("intruder")));
        assert_eq!(r.status, InvariantStatus::Fail);
        assert!(r.cause.as_ref().is_some_and(|c| c.contains("not in the participant set")));
    }

    #[test]
    fn leader_set_with_exactly_one_entry_passes() {
        let leaders = vec![n("host-0")];
        let r = evaluate_single_leader_from_leader_set(&leaders);
        assert_eq!(r.status, InvariantStatus::Pass);
    }

    #[test]
    fn empty_leader_set_fails() {
        let leaders: Vec<NodeId> = Vec::new();
        let r = evaluate_single_leader_from_leader_set(&leaders);
        assert_eq!(r.status, InvariantStatus::Fail);
    }

    #[test]
    fn two_leaders_fail_with_count_in_cause() {
        let leaders = vec![n("host-0"), n("host-1")];
        let r = evaluate_single_leader_from_leader_set(&leaders);
        assert_eq!(r.status, InvariantStatus::Fail);
        assert!(r.cause.as_ref().is_some_and(|c| c.contains('2')));
    }

    #[test]
    fn entropy_determinism_reports_pass_on_equal_seeds() {
        assert_eq!(evaluate_entropy_determinism(7).status, InvariantStatus::Pass);
    }

    #[test]
    fn entropy_determinism_reports_fail_on_divergent_streams() {
        let a = SimEntropy::new(1);
        let b = SimEntropy::new(2);
        assert_eq!(evaluate_entropy_determinism_against(&a, &b).status, InvariantStatus::Fail);
    }

    // workflow-primitive step 01-07 graduated the
    // `ReplayEquivalentEmptyWorkflow` two-SimEntropy-transcripts
    // placeholder into a real journal replay (the
    // `evaluate_replay_equivalence_provision_record` evaluator + its two
    // sibling workflow durability evaluators). The transcript-stub unit
    // tests that defended the deleted placeholder are gone with it; the
    // graduated evaluators are exercised through
    // `tests/invariant_evaluators.rs` (direct, async) and
    // `tests/acceptance/replay_equivalence_provision_record_invariant.rs`
    // (port-to-port through the harness).

    // -----------------------------------------------------------------
    // Step 04-05 — reconciler-primitive invariant witnesses
    // -----------------------------------------------------------------

    #[test]
    fn at_least_one_reconciler_passes_on_nonzero_count() {
        assert_eq!(evaluate_at_least_one_reconciler_registered(1).status, InvariantStatus::Pass,);
        assert_eq!(evaluate_at_least_one_reconciler_registered(42).status, InvariantStatus::Pass,);
    }

    #[test]
    fn at_least_one_reconciler_fails_on_empty_registry() {
        let r = evaluate_at_least_one_reconciler_registered(0);
        assert_eq!(r.status, InvariantStatus::Fail);
        assert!(r.cause.as_ref().is_some_and(|c| c.contains("empty")));
    }

    #[test]
    fn duplicate_evaluations_collapse_passes_on_clean_3_way_collapse() {
        let counters = BrokerCountersSnapshot { queued: 0, cancelled: 2, dispatched: 1 };
        assert_eq!(
            evaluate_duplicate_evaluations_collapse(3, counters).status,
            InvariantStatus::Pass,
        );
    }

    #[test]
    fn duplicate_evaluations_collapse_fails_when_dispatched_not_one() {
        // dispatched == 2 means the second submit didn't supersede the
        // first — key-collapse is broken.
        let counters = BrokerCountersSnapshot { queued: 0, cancelled: 1, dispatched: 2 };
        let r = evaluate_duplicate_evaluations_collapse(3, counters);
        assert_eq!(r.status, InvariantStatus::Fail);
    }

    #[test]
    fn duplicate_evaluations_collapse_fails_when_cancelled_count_wrong() {
        // N=3 should yield cancelled=2; cancelled=0 means nothing was
        // actually superseded.
        let counters = BrokerCountersSnapshot { queued: 0, cancelled: 0, dispatched: 1 };
        let r = evaluate_duplicate_evaluations_collapse(3, counters);
        assert_eq!(r.status, InvariantStatus::Fail);
    }

    #[test]
    fn duplicate_evaluations_collapse_fails_when_queued_not_drained() {
        // queued > 0 means the drain half of the sequence never ran.
        let counters = BrokerCountersSnapshot { queued: 1, cancelled: 2, dispatched: 0 };
        let r = evaluate_duplicate_evaluations_collapse(3, counters);
        assert_eq!(r.status, InvariantStatus::Fail);
    }

    #[test]
    fn duplicate_evaluations_collapse_fails_when_n_below_three() {
        // Invariant requires at least 3 submitted to be meaningful.
        let counters = BrokerCountersSnapshot { queued: 0, cancelled: 1, dispatched: 1 };
        let r = evaluate_duplicate_evaluations_collapse(2, counters);
        assert_eq!(r.status, InvariantStatus::Fail);
        assert!(r.cause.as_ref().is_some_and(|c| c.contains("at least 3")));
    }

    // -----------------------------------------------------------------
    // Step 01-05 — BrokerDrainOrderIsDeterministic witnesses
    // -----------------------------------------------------------------

    /// Build a small fixture of `(ReconcilerName, TargetResource)` pairs
    /// for the broker-drain-order tests. Two entries is the minimum that
    /// can demonstrate divergence at a non-trivial position; 01-05 keeps
    /// the fixture deliberately tiny so the failure-message assertion
    /// pins the position index, not incidental ordering.
    fn drain_fixture() -> Vec<(
        overdrive_core::reconcilers::ReconcilerName,
        overdrive_core::reconcilers::TargetResource,
    )> {
        use overdrive_core::reconcilers::{ReconcilerName, TargetResource};
        let r = ReconcilerName::new("noop-heartbeat")
            .expect("noop-heartbeat is a valid ReconcilerName");
        let t_a =
            TargetResource::new("job/payments").expect("job/payments is a valid TargetResource");
        let t_b =
            TargetResource::new("job/frontend").expect("job/frontend is a valid TargetResource");
        vec![(r.clone(), t_a), (r, t_b)]
    }

    #[test]
    fn evaluate_broker_drain_order_is_deterministic_pass_and_fail() {
        // PASS: two snapshots with identical dispatched_order vecs.
        let order = drain_fixture();
        let a = BrokerDrainOrderSnapshot { dispatched_order: order.clone() };
        let b = BrokerDrainOrderSnapshot { dispatched_order: order.clone() };
        let pass = evaluate_broker_drain_order_is_deterministic(&a, &b);
        assert_eq!(pass.status, InvariantStatus::Pass);

        // FAIL: divergence at position 0 — swap the first pair so the
        // first index differs. The failure message must name the
        // divergent position.
        let mut divergent = order.clone();
        divergent.swap(0, 1);
        let a2 = BrokerDrainOrderSnapshot { dispatched_order: order };
        let b2 = BrokerDrainOrderSnapshot { dispatched_order: divergent };
        let fail = evaluate_broker_drain_order_is_deterministic(&a2, &b2);
        assert_eq!(fail.status, InvariantStatus::Fail);
        // Structured assertion: the failure message must name the first
        // divergent position by index. Position 0 is where the swap
        // takes effect.
        assert!(
            fail.cause.as_ref().is_some_and(|c| c.contains("position 0")),
            "failure message must name divergent position; got {:?}",
            fail.cause,
        );
    }

    // -----------------------------------------------------------------
    // fix-dst-dispatch-routing-invariant 01-01 — DispatchRoutingIsNameRestricted witnesses
    //
    // Cover all four evaluator branches: (a) vacuous-pass on empty
    // input, (b) cardinality, (c) per-eval routing, (d) smoking-gun.
    // Mutations on any branch predicate are killed by at least one of
    // these witnesses — paired with the end-to-end tests under
    // `tests/invariant_evaluators.rs` (the regression-test proof).
    // -----------------------------------------------------------------

    fn dispatch_jl_reconciler() -> overdrive_core::reconcilers::ReconcilerName {
        overdrive_core::reconcilers::ReconcilerName::new("job-lifecycle")
            .expect("job-lifecycle is a valid ReconcilerName")
    }

    fn dispatch_noop_reconciler() -> overdrive_core::reconcilers::ReconcilerName {
        overdrive_core::reconcilers::ReconcilerName::new("noop-heartbeat")
            .expect("noop-heartbeat is a valid ReconcilerName")
    }

    fn dispatch_target(raw: &str) -> overdrive_core::reconcilers::TargetResource {
        overdrive_core::reconcilers::TargetResource::new(raw).expect("valid TargetResource")
    }

    /// Branch (c, d) — happy path single eval.
    #[test]
    fn dispatch_routing_passes_on_clean_single_eval() {
        let r = dispatch_jl_reconciler();
        let t = dispatch_target("job/payments");
        let submitted = vec![Evaluation { reconciler: r.clone(), target: t.clone() }];
        let record = DispatchRecord { dispatched: vec![(r, t)] };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Pass);
        assert!(result.cause.is_none());
    }

    /// Branch (c, d) — happy path multi eval, distinct targets.
    #[test]
    fn dispatch_routing_passes_on_clean_multi_eval_distinct_targets() {
        let r = dispatch_jl_reconciler();
        let t_a = dispatch_target("job/payments");
        let t_b = dispatch_target("job/frontend");
        let submitted = vec![
            Evaluation { reconciler: r.clone(), target: t_a.clone() },
            Evaluation { reconciler: r.clone(), target: t_b.clone() },
        ];
        let record = DispatchRecord { dispatched: vec![(r.clone(), t_a), (r, t_b)] };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Pass);
    }

    /// Branch (a) — vacuous-pass on empty input.
    #[test]
    fn dispatch_routing_passes_vacuously_on_empty_input() {
        let submitted: Vec<Evaluation> = Vec::new();
        let record = DispatchRecord { dispatched: Vec::new() };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Pass);
    }

    /// Branch (b) — cardinality fail: dispatched has more entries than
    /// submitted.
    #[test]
    fn dispatch_routing_fails_on_cardinality_mismatch_extra() {
        let r = dispatch_jl_reconciler();
        let noop = dispatch_noop_reconciler();
        let t = dispatch_target("job/payments");
        let submitted = vec![Evaluation { reconciler: r.clone(), target: t.clone() }];
        // Two dispatch entries for one drained eval — fan-out shape.
        let record = DispatchRecord { dispatched: vec![(r, t.clone()), (noop, t)] };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Fail);
        assert!(
            result
                .cause
                .as_ref()
                .is_some_and(|c| c.contains("expected") && c.contains("dispatch entries")),
            "cause must name cardinality shape; got {:?}",
            result.cause,
        );
    }

    /// Branch (b) — cardinality fail: dispatched has fewer entries
    /// than submitted (missed dispatch).
    #[test]
    fn dispatch_routing_fails_on_cardinality_mismatch_missing() {
        let r = dispatch_jl_reconciler();
        let t_a = dispatch_target("job/payments");
        let t_b = dispatch_target("job/frontend");
        let submitted = vec![
            Evaluation { reconciler: r.clone(), target: t_a.clone() },
            Evaluation { reconciler: r.clone(), target: t_b },
        ];
        let record = DispatchRecord { dispatched: vec![(r, t_a)] };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Fail);
        assert!(
            result
                .cause
                .as_ref()
                .is_some_and(|c| c.contains("expected") && c.contains("dispatch entries")),
            "cause must name cardinality shape; got {:?}",
            result.cause,
        );
    }

    /// Branch (d) — smoking-gun: cardinality matches but one dispatch
    /// entry names an unsubmitted reconciler. Pinning per-eval routing
    /// (branch c) catches this first; the smoking-gun branch is
    /// reachable when per-eval routing PASSES but the dispatcher
    /// somehow added a stray entry of the same length. To exercise
    /// branch (d) deterministically we construct a fixture where (c)
    /// finds zero matches for the second submitted eval and fails —
    /// this still proves the fan-out shape is rejected.
    #[test]
    fn dispatch_routing_fails_on_smoking_gun_unsubmitted_reconciler() {
        let jl = dispatch_jl_reconciler();
        let noop = dispatch_noop_reconciler();
        let t_a = dispatch_target("job/payments");
        let t_b = dispatch_target("job/frontend");
        // Two submitted evals naming `jl`. Dispatched has matching
        // cardinality (2) but the SECOND entry names `noop` — an
        // unsubmitted reconciler. Per-eval routing fails on the second
        // submitted eval (zero matches for `(jl, t_b)`); the cause
        // names the fan-out shape.
        let submitted = vec![
            Evaluation { reconciler: jl.clone(), target: t_a.clone() },
            Evaluation { reconciler: jl.clone(), target: t_b.clone() },
        ];
        let record = DispatchRecord { dispatched: vec![(jl, t_a), (noop, t_b)] };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Fail);
        // Either per-eval routing or smoking-gun cause is acceptable;
        // both name the wrong-reconciler shape. Per-eval routing fires
        // first under the current branch order.
        assert!(
            result.cause.as_ref().is_some_and(|c| c.contains("expected exactly one dispatch")),
            "per-eval routing cause must name the named-reconciler shape; got {:?}",
            result.cause,
        );
    }

    /// Branch (c) — wrong-routing: cardinality matches but dispatcher
    /// invoked a different reconciler than the one submitted.
    #[test]
    fn dispatch_routing_fails_when_named_reconciler_not_dispatched() {
        let jl = dispatch_jl_reconciler();
        let noop = dispatch_noop_reconciler();
        let t = dispatch_target("job/payments");
        let submitted = vec![Evaluation { reconciler: jl, target: t.clone() }];
        // Cardinality matches (1 == 1) but the dispatched reconciler
        // is wrong. Per-eval routing finds zero matches for `(jl, t)`.
        let record = DispatchRecord { dispatched: vec![(noop, t)] };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Fail);
    }

    /// Pure smoking-gun branch (d) — only reachable when per-eval
    /// routing has already passed for every submitted eval AND there
    /// is still an extra dispatched entry naming an unsubmitted
    /// reconciler. The cardinality branch catches the surplus first
    /// under the current shape, but if a future refactor reorders the
    /// branches, the smoking-gun assertion must still fire on this
    /// fixture. Constructed to fail at the cardinality branch with a
    /// cause that names the surplus.
    #[test]
    fn dispatch_routing_fails_when_extra_entry_names_unsubmitted_reconciler() {
        let jl = dispatch_jl_reconciler();
        let noop = dispatch_noop_reconciler();
        let t_a = dispatch_target("job/payments");
        let t_b = dispatch_target("job/frontend");
        let submitted = vec![Evaluation { reconciler: jl.clone(), target: t_a.clone() }];
        // One submitted eval, two dispatched entries — the extra one
        // names `noop`, an unsubmitted reconciler.
        let record = DispatchRecord { dispatched: vec![(jl, t_a), (noop, t_b)] };
        let result = evaluate_dispatch_routing_is_name_restricted(&submitted, &record);
        assert_eq!(result.status, InvariantStatus::Fail);
    }

    #[test]
    fn build_tick_context_produces_deadline_strictly_after_now() {
        // Kills the `+ with -` mutation on the deadline arithmetic in
        // `build_tick_context`: with `-`, `now - BUDGET` would produce
        // a deadline in the past (before `now`), failing this
        // assertion. With the original `+`, deadline is exactly
        // `BUDGET` ahead of `now`. The Phase 1 reconciler
        // (`NoopHeartbeat`) ignores `tick.deadline`, so without a
        // direct test on the helper the mutation survives — the
        // evaluator returns a deterministic Pass either way.
        let clock = SimClock::new();
        let tick = build_tick_context(&clock);
        assert!(
            tick.deadline > tick.now,
            "deadline must be strictly after now; got now={:?} deadline={:?}",
            tick.now,
            tick.deadline,
        );
        assert_eq!(tick.tick, 0, "tick counter is a fixed zero per the evaluator contract");
    }

    #[test]
    fn reconciler_is_pure_passes_for_deterministic_reconciler() {
        use overdrive_core::reconcilers::{AnyReconciler, NoopHeartbeat};

        // The deterministic witness is the real `NoopHeartbeat` —
        // wrapping it in `AnyReconciler::NoopHeartbeat` exercises the
        // exact enum-dispatch path the evaluator runs in production.
        let r = AnyReconciler::NoopHeartbeat(NoopHeartbeat::canonical());
        let clock = SimClock::new();
        assert_eq!(evaluate_reconciler_is_pure(&r, &clock).status, InvariantStatus::Pass);
    }

    // -----------------------------------------------------------------
    // phase-1-first-workload — slice 3 (US-03) — convergence invariants
    // -----------------------------------------------------------------

    /// Build an `AllocStatusRow` fixture for the convergence-invariant
    /// witnesses. Centralises the boilerplate so individual tests stay
    /// focused on their assertion.
    fn alloc_row(alloc: &str, job: &str, node: &str, state: AllocState) -> AllocStatusRow {
        use overdrive_core::id::AllocationId;
        use overdrive_core::traits::observation_store::LogicalTimestamp;
        AllocStatusRow {
            alloc_id: AllocationId::new(alloc).expect("valid alloc id"),
            workload_id: WorkloadId::new(job).expect("valid job id"),
            node_id: NodeId::new(node).expect("valid node id"),
            state,
            updated_at: LogicalTimestamp {
                counter: 1,
                writer: NodeId::new(node).expect("valid node id"),
            },
            reason: None,
            detail: None,
            terminal: None,
            stderr_tail: None,
            kind: overdrive_core::aggregate::WorkloadKind::Service,
            listeners: Vec::new(),
            // GAP-1 subsidiary: None on Pending; fixed wall-clock otherwise.
            started_at: match state {
                AllocState::Pending => None,
                _ => Some(UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000))),
            },
        }
    }

    #[test]
    fn job_scheduled_after_submission_passes_when_every_job_has_running_alloc() {
        let jobs = vec![WorkloadId::new("payments").expect("valid job id")];
        let rows = vec![alloc_row("alloc-payments-0", "payments", "node-1", AllocState::Running)];
        let r = evaluate_workload_scheduled_after_submission(&jobs, &rows);
        assert_eq!(r.status, InvariantStatus::Pass);
    }

    #[test]
    fn job_scheduled_after_submission_fails_when_no_running_alloc_for_submitted_job() {
        let jobs = vec![WorkloadId::new("payments").expect("valid job id")];
        let rows = vec![alloc_row("alloc-payments-0", "payments", "node-1", AllocState::Pending)];
        let r = evaluate_workload_scheduled_after_submission(&jobs, &rows);
        assert_eq!(r.status, InvariantStatus::Fail);
        assert!(r.cause.as_ref().is_some_and(|c| c.contains("no Running alloc")));
    }

    #[test]
    fn job_scheduled_after_submission_passes_vacuously_with_no_submissions() {
        let jobs: Vec<WorkloadId> = Vec::new();
        let rows: Vec<AllocStatusRow> = Vec::new();
        let r = evaluate_workload_scheduled_after_submission(&jobs, &rows);
        assert_eq!(r.status, InvariantStatus::Pass);
    }

    #[test]
    fn job_scheduled_after_submission_fails_when_running_alloc_belongs_to_different_job() {
        let jobs = vec![WorkloadId::new("payments").expect("valid job id")];
        let rows = vec![alloc_row("alloc-frontend-0", "frontend", "node-1", AllocState::Running)];
        let r = evaluate_workload_scheduled_after_submission(&jobs, &rows);
        assert_eq!(r.status, InvariantStatus::Fail);
    }

    #[test]
    fn desired_replica_count_converges_passes_at_n_equals_one() {
        let want = vec![(WorkloadId::new("payments").expect("valid job id"), 1)];
        let rows = vec![alloc_row("alloc-payments-0", "payments", "node-1", AllocState::Running)];
        let r = evaluate_desired_replica_count_converges(&want, &rows);
        assert_eq!(r.status, InvariantStatus::Pass);
    }

    #[test]
    fn desired_replica_count_converges_fails_when_observed_count_undershoots() {
        let want = vec![(WorkloadId::new("payments").expect("valid job id"), 2)];
        let rows = vec![alloc_row("alloc-payments-0", "payments", "node-1", AllocState::Running)];
        let r = evaluate_desired_replica_count_converges(&want, &rows);
        assert_eq!(r.status, InvariantStatus::Fail);
        assert!(r.cause.as_ref().is_some_and(|c| c.contains("want 2")));
    }

    #[test]
    fn desired_replica_count_converges_fails_when_observed_count_overshoots() {
        let want = vec![(WorkloadId::new("payments").expect("valid job id"), 1)];
        let rows = vec![
            alloc_row("alloc-payments-0", "payments", "node-1", AllocState::Running),
            alloc_row("alloc-payments-1", "payments", "node-1", AllocState::Running),
        ];
        let r = evaluate_desired_replica_count_converges(&want, &rows);
        assert_eq!(r.status, InvariantStatus::Fail);
    }

    /// Pin the `&&` clause in the row filter against `||`. The
    /// fixture mixes a Running alloc for the target job with a
    /// non-Running alloc for the SAME job — under `&&` only the
    /// Running one counts (count=1, equals desired); under `||`
    /// both clauses are independently true so both rows count
    /// (count=2, mismatches desired). The `Pass` outcome is unique
    /// to the `&&` shape.
    #[test]
    fn desired_replica_count_converges_distinguishes_and_from_or_in_row_filter() {
        let want = vec![(WorkloadId::new("payments").expect("valid job id"), 1)];
        // Two rows for the SAME job: one Running, one Terminated.
        // Under production `&&`: only Running matches → count=1 ⇒
        // matches desired (1) ⇒ Pass.
        // Under mutant `||`: both match (first via state==Running,
        // second via workload_id==payments) → count=2 ⇒ mismatches
        // desired (1) ⇒ Fail.
        let rows = vec![
            alloc_row("alloc-payments-0", "payments", "node-1", AllocState::Running),
            alloc_row("alloc-payments-1", "payments", "node-1", AllocState::Terminated),
        ];
        let r = evaluate_desired_replica_count_converges(&want, &rows);
        assert_eq!(
            r.status,
            InvariantStatus::Pass,
            "`&&` filter must count only Running allocs of target job; got {r:?}",
        );
    }

    #[test]
    fn no_double_scheduling_passes_on_consistent_pinning() {
        let rows = vec![
            alloc_row("alloc-a", "payments", "node-1", AllocState::Running),
            alloc_row("alloc-b", "frontend", "node-2", AllocState::Running),
            // Same alloc, same node — duplicate row from gossip is not
            // a violation.
            alloc_row("alloc-a", "payments", "node-1", AllocState::Running),
        ];
        let r = evaluate_no_double_scheduling(&rows);
        assert_eq!(r.status, InvariantStatus::Pass);
    }

    #[test]
    fn no_double_scheduling_fails_when_alloc_pinned_to_two_nodes() {
        let rows = vec![
            alloc_row("alloc-a", "payments", "node-1", AllocState::Running),
            alloc_row("alloc-a", "payments", "node-2", AllocState::Running),
        ];
        let r = evaluate_no_double_scheduling(&rows);
        assert_eq!(r.status, InvariantStatus::Fail);
        assert!(r.cause.as_ref().is_some_and(|c| c.contains("two nodes")));
    }

    #[test]
    fn no_double_scheduling_passes_on_empty_snapshot() {
        let rows: Vec<AllocStatusRow> = Vec::new();
        let r = evaluate_no_double_scheduling(&rows);
        assert_eq!(r.status, InvariantStatus::Pass);
    }

    // -----------------------------------------------------------------
    // reconciler-memory-redb step 01-07 — ViewStore DST invariant
    // witnesses. Library-level pins for each new evaluator. Paired with
    // the acceptance tests under
    // `tests/acceptance/reconciler_invariants_pass.rs`.
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn view_store_roundtrip_is_lossless_passes_on_clean_store() {
        // Same seed twice MUST produce the same verdict (K3
        // reproducibility). Pass on a clean store proves the
        // proptest-driven body generates roundtrippable values.
        let r1 = evaluate_view_store_roundtrip_is_lossless(42).await;
        let r2 = evaluate_view_store_roundtrip_is_lossless(42).await;
        assert_eq!(r1.status, InvariantStatus::Pass, "first run must pass; got {r1:?}");
        assert_eq!(r2.status, InvariantStatus::Pass, "second run must pass; got {r2:?}");
        assert_eq!(r1.cause, r2.cause, "deterministic seed must produce identical cause");
    }

    #[tokio::test]
    async fn view_store_roundtrip_is_lossless_covers_unit_and_jobview() {
        // The evaluator covers BOTH `WorkloadLifecycleView` and `()`. A
        // pass result proves both code paths fired without error —
        // the body returns Fail on the FIRST roundtrip divergence
        // (across WorkloadLifecycleView cases or the unit-View case), so
        // a green verdict implies every covered shape roundtripped.
        let r = evaluate_view_store_roundtrip_is_lossless(7).await;
        assert_eq!(r.status, InvariantStatus::Pass, "covers both View shapes; got {r:?}");
    }

    #[tokio::test]
    async fn bulk_load_is_deterministic_passes_on_seeded_corpus() {
        // The evaluator pre-populates ≥3 entries spanning the
        // job-lifecycle reconciler and asserts two reads produce
        // PartialEq-equal maps. On the BTreeMap-backed SimViewStore
        // this is structurally true — the test catches a regression
        // that would swap BTreeMap for HashMap or otherwise
        // destabilise iteration order.
        let r = evaluate_bulk_load_is_deterministic().await;
        assert_eq!(r.status, InvariantStatus::Pass, "clean store passes; got {r:?}");
    }

    #[tokio::test]
    async fn bulk_load_is_deterministic_repeats_pass() {
        // Repeating the evaluator must produce the same Pass verdict
        // — no hidden state in the evaluator that could leak across
        // runs.
        for _ in 0..3 {
            let r = evaluate_bulk_load_is_deterministic().await;
            assert_eq!(r.status, InvariantStatus::Pass);
        }
    }

    #[tokio::test]
    async fn write_through_ordering_passes_when_runtime_obeys_fsync_first() {
        // The runtime fsyncs THEN updates the in-memory map per
        // ADR-0035 §5. Under fsync injection, the in-memory map
        // MUST still hold the pre-injection value. Pass on the
        // clean default runtime build proves the invariant fires
        // green when the contract holds.
        let r = evaluate_write_through_ordering().await;
        assert_eq!(r.status, InvariantStatus::Pass, "fsync-first runtime passes; got {r:?}");
    }

    #[tokio::test]
    async fn write_through_ordering_emits_canonical_name() {
        // The result name MUST match the canonical kebab-case form
        // — DST summary parsers (CI, dst-summary.json schema) key on
        // this string.
        let r = evaluate_write_through_ordering().await;
        assert_eq!(r.name, "write-through-ordering");
    }

    #[tokio::test]
    async fn view_store_roundtrip_emits_canonical_name() {
        let r = evaluate_view_store_roundtrip_is_lossless(1).await;
        assert_eq!(r.name, "view-store-roundtrip-is-lossless");
    }

    #[tokio::test]
    async fn bulk_load_is_deterministic_emits_canonical_name() {
        let r = evaluate_bulk_load_is_deterministic().await;
        assert_eq!(r.name, "bulk-load-is-deterministic");
    }

    // -----------------------------------------------------------------
    // Pure helper unit tests — kill mutations on the comparison guards
    // inside the larger async evaluators.
    //
    // The evaluator bodies always pass on a clean SimViewStore, so a
    // mutation that flips `==` to `!=` (or replaces a guard with
    // `true`/`false`) on the verdict comparison goes uncaught at the
    // evaluator level — every input matches by construction. Splitting
    // the comparison into a typed `RoundtripVerdict` /
    // `OrderingVerdict` pair lets these unit tests pass DELIBERATELY-
    // mismatched inputs and assert the right verdict variant fires.
    // -----------------------------------------------------------------

    #[test]
    fn roundtrip_verdict_returns_match_when_decoded_equals_expected() {
        let v: u32 = 42;
        assert_eq!(roundtrip_verdict(Some(&v), &v), RoundtripVerdict::Match);
    }

    #[test]
    fn roundtrip_verdict_returns_mismatch_when_decoded_differs() {
        let decoded: u32 = 1;
        let expected: u32 = 2;
        assert_eq!(roundtrip_verdict(Some(&decoded), &expected), RoundtripVerdict::Mismatch);
    }

    #[test]
    fn roundtrip_verdict_returns_absent_when_no_decoded_value() {
        let expected: u32 = 42;
        let absent: Option<&u32> = None;
        assert_eq!(roundtrip_verdict(absent, &expected), RoundtripVerdict::Absent);
    }

    #[test]
    fn roundtrip_verdict_distinguishes_equal_from_not_equal_for_jobview() {
        // Pin against a `==` flip. Two structurally-different
        // WorkloadLifecycleView values must produce Mismatch; identical
        // ones must produce Match. A mutation that flips the inner
        // PartialEq comparison surfaces here.
        use overdrive_core::id::AllocationId;
        use overdrive_core::reconcilers::WorkloadLifecycleView;

        let mut a = WorkloadLifecycleView::default();
        a.restart_counts.insert(AllocationId::new("alloc-a").expect("valid"), 1);
        let mut b = WorkloadLifecycleView::default();
        b.restart_counts.insert(AllocationId::new("alloc-a").expect("valid"), 2);

        assert_eq!(roundtrip_verdict(Some(&a), &a), RoundtripVerdict::Match);
        assert_eq!(roundtrip_verdict(Some(&a), &b), RoundtripVerdict::Mismatch);
    }

    #[test]
    fn ordering_verdict_returns_held_when_loaded_equals_original() {
        let original: u32 = 1;
        let next: u32 = 2;
        assert_eq!(
            ordering_verdict(Some(&original), &original, &next),
            OrderingVerdict::OrderingHeld,
        );
    }

    #[test]
    fn ordering_verdict_returns_advanced_when_loaded_equals_next_view() {
        let original: u32 = 1;
        let next: u32 = 2;
        // Loaded carries the would-be next_view — the runtime advanced
        // its in-memory map despite the fsync failure. This is the
        // exact ordering violation the invariant catches.
        assert_eq!(ordering_verdict(Some(&next), &original, &next), OrderingVerdict::Advanced);
    }

    #[test]
    fn ordering_verdict_returns_unexpected_when_loaded_matches_neither() {
        let original: u32 = 1;
        let next: u32 = 2;
        let other: u32 = 99;
        assert_eq!(
            ordering_verdict(Some(&other), &original, &next),
            OrderingVerdict::UnexpectedView,
        );
    }

    #[test]
    fn ordering_verdict_returns_absent_when_no_loaded_value() {
        let original: u32 = 1;
        let next: u32 = 2;
        let absent: Option<&u32> = None;
        assert_eq!(ordering_verdict(absent, &original, &next), OrderingVerdict::Absent);
    }

    #[test]
    fn ordering_verdict_distinguishes_held_from_advanced_under_jobview() {
        // Under realistic WorkloadLifecycleView shapes, OrderingHeld and
        // Advanced must remain distinguishable. A mutation flipping
        // the `==` on either guard would collapse one of these two
        // assertions.
        use overdrive_core::id::AllocationId;
        use overdrive_core::reconcilers::WorkloadLifecycleView;

        let id = AllocationId::new("alloc-payments-0").expect("valid");
        let mut original = WorkloadLifecycleView::default();
        original.restart_counts.insert(id.clone(), 7);
        let mut next = WorkloadLifecycleView::default();
        next.restart_counts.insert(id, 99);

        assert_eq!(
            ordering_verdict(Some(&original), &original, &next),
            OrderingVerdict::OrderingHeld,
        );
        assert_eq!(ordering_verdict(Some(&next), &original, &next), OrderingVerdict::Advanced,);
    }

    // ---------------------------------------------------------------------
    // Step 01-06 (D6 / ADR-0064 §6) — the EXTENDED replay-equivalence guard
    // helpers. These prove the guards BITE: a dropped `Started` at command-
    // index 0 fails (b); a `SignalSeen` consumed as a command fails (c).
    // ---------------------------------------------------------------------

    use overdrive_core::id::ContentHash;
    use overdrive_core::workflow::{SignalKey, SignalValue};

    /// A `Started` command-index-0 entry.
    fn started() -> LoadedEntry {
        let d = ContentHash::of(b"spec");
        LoadedEntry::Command(JournalCommand::Started { spec_digest: d, input_digest: d })
    }

    /// A `RunResult` command with the given name + result bytes.
    fn run_result(name: &str, bytes: &[u8]) -> LoadedEntry {
        LoadedEntry::Command(JournalCommand::RunResult {
            name: name.to_owned(),
            result_digest: ContentHash::of(bytes),
            result_bytes: bytes.to_vec(),
        })
    }

    /// A `Terminal` command carrying a `WorkflowStatus::Completed` with an
    /// empty erased `Output` — the contentless-success terminal the
    /// reference `ProvisionRecord` fixture (`Output = ()`) projects to under
    /// the migrated terminal model (ADR-0065 §3). These guard tests compare
    /// entry-KIND sequences, so the opaque `output` bytes are immaterial; an
    /// empty `Vec` keeps the fixture minimal.
    fn terminal() -> LoadedEntry {
        LoadedEntry::Command(JournalCommand::Terminal {
            status: WorkflowStatus::Completed { output: Vec::new() },
        })
    }

    /// A `SignalAwaited` command for `key`.
    fn signal_awaited(key: &SignalKey) -> LoadedEntry {
        LoadedEntry::Command(JournalCommand::SignalAwaited { signal_key: key.clone() })
    }

    /// An `ActionEmitted` command.
    fn action_emitted() -> LoadedEntry {
        LoadedEntry::Command(JournalCommand::ActionEmitted { action_digest: ContentHash::of(b"a") })
    }

    /// A `SignalSeen` NOTIFICATION for `key` (off the positional walk).
    fn signal_seen(key: &SignalKey) -> LoadedEntry {
        LoadedEntry::Notification(JournalNotification::SignalSeen {
            signal_key: key.clone(),
            value_digest: ContentHash::of(b"v"),
            value: SignalValue::new("v"),
        })
    }

    fn sig_key() -> SignalKey {
        SignalKey::new("provision-signal").expect("valid signal key")
    }

    // ---- (b) Started-at-index-0 full-command-sequence equality ----

    #[test]
    fn b_guard_passes_when_both_runs_begin_with_started_and_sequences_match() {
        let uninterrupted = vec![started(), run_result("provision-write", b"ok"), terminal()];
        let resumed = uninterrupted.clone();
        assert_eq!(
            assert_started_at_index_0_and_command_sequence_identical(&resumed, &uninterrupted),
            None,
            "identical runs both opening with Started pass the guard",
        );
    }

    #[test]
    fn b_guard_bites_when_resumed_run_drops_started_at_index_0() {
        // The TRAP: the resumed run's command sequence does NOT begin with
        // `Started` (a dropped engine.start Started write). The guard MUST
        // fail.
        let uninterrupted = vec![started(), run_result("provision-write", b"ok"), terminal()];
        let resumed = vec![run_result("provision-write", b"ok"), terminal()];
        let cause =
            assert_started_at_index_0_and_command_sequence_identical(&resumed, &uninterrupted)
                .expect("a resumed run missing Started at index 0 MUST fail the guard");
        assert!(
            cause.contains("does not begin with `Started`"),
            "the cause names the dropped Started write: {cause}",
        );
    }

    #[test]
    fn b_guard_bites_when_command_sequences_diverge() {
        // Both open with Started, but the resumed sequence diverges (an extra
        // command). The full-sequence equality MUST fail (the narrow prior
        // "recorded RunResult matches" equality would have missed this).
        let uninterrupted = vec![started(), run_result("provision-write", b"ok"), terminal()];
        let resumed = vec![
            started(),
            run_result("provision-write", b"ok"),
            run_result("extra", b"x"),
            terminal(),
        ];
        let cause =
            assert_started_at_index_0_and_command_sequence_identical(&resumed, &uninterrupted)
                .expect("a divergent command sequence MUST fail the guard");
        assert!(
            cause.contains("diverges from uninterrupted"),
            "the cause names the divergence: {cause}",
        );
    }

    // ---- (c) notification-not-as-command cursor-advance guard ----

    #[tokio::test]
    async fn c_guard_passes_when_signal_seen_is_off_the_command_walk() {
        // The signal+emit shape: SignalSeen is a NOTIFICATION interleaved in
        // the run, resolved by SignalKey OFF the positional walk. The command
        // walk is [Started, SignalAwaited, ActionEmitted, Terminal]. The
        // cursor walks commands only; the guard PASSES.
        let key = sig_key();
        let run = vec![
            started(),
            signal_awaited(&key),
            signal_seen(&key), // the notification — off the walk
            action_emitted(),
            terminal(),
        ];
        assert_eq!(
            assert_notification_not_as_command(&run).await,
            None,
            "a SignalSeen resolved by key (off the walk) passes the guard",
        );
    }

    #[tokio::test]
    async fn c_guard_is_not_vacuous_on_a_notification_free_run() {
        // The guard requires ≥1 interleaved notification so it genuinely
        // exercises the off-the-walk lookup — it does NOT vacuously pass a
        // notification-free run (which would be a meaningless green).
        let key = sig_key();
        let run = vec![started(), signal_awaited(&key), action_emitted(), terminal()];
        let cause = assert_notification_not_as_command(&run)
            .await
            .expect("a run with no off-the-walk notification MUST NOT pass the guard");
        assert!(
            cause.contains("requires a run with an interleaved SignalSeen"),
            "a notification-free run trips the precondition arm: {cause}",
        );
    }

    #[tokio::test]
    async fn c_guard_bites_when_notification_present_but_not_resolvable_by_key() {
        // The TRAP's TWIN, sharper: a SignalSeen notification IS present in
        // the run (so the precondition is satisfied) but its SignalKey does
        // NOT match the SignalAwaited command's key — modelling a notification
        // that the cursor cannot resolve off the walk because the correlation
        // broke (equivalent to it being mis-routed into the command walk
        // rather than the key-correlated map). replay_signal(awaited_key)
        // returns Ok(None) (no matching SignalSeen by key) → the guard fails.
        let awaited_key = sig_key();
        let other_key = SignalKey::new("unrelated-signal").expect("valid");
        let run = vec![
            started(),
            signal_awaited(&awaited_key),
            signal_seen(&other_key), // present, but keyed to a DIFFERENT signal
            action_emitted(),
            terminal(),
        ];
        let cause = assert_notification_not_as_command(&run)
            .await
            .expect("a notification not resolvable by the awaited key MUST fail the guard");
        assert!(
            cause.contains("did not resolve the recorded SignalSeen by key"),
            "the cause names the off-the-walk resolution failure: {cause}",
        );
    }
}
