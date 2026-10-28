//! `ServiceLifecycleView` under PROBE-RUN-COUNT (netns-density-295,
//! ADR-0191): the view keeps no probe streak. The row's `same_status_runs`
//! is the streak, and the only probe-count memory the view carries is
//! `liveness_last_acted_run` — the `run_seq` of the row each liveness key
//! last terminated on (act once per crossing).
//!
//! The view is runtime-persisted CBOR with no `deny_unknown_fields`
//! (`.claude/rules/development.md` § "Reconciler I/O" → "Schema
//! evolution"): a view persisted before the change still decodes, its
//! removed streak fields are dropped, and the new field defaults to empty.
//!
//! The composed behaviour — the probe runner's run numbering, the merge and
//! the read-time threshold rule driven through the reconciler runtime — is
//! R5-PROBE-COUNT in `overdrive-control-plane`
//! (`src/reconciler_runtime/probe_run_count.rs`) and the `probe-count`
//! conformance driver beside it.

#![allow(clippy::doc_markdown, reason = "per-test CONTRACT_SHAPE declarations")]

use std::collections::{BTreeMap, BTreeSet};

use overdrive_core::id::AllocationId;
use overdrive_core::observation::{ProbeIdx, ProbeRunSeq};
use overdrive_reconcilers::service_lifecycle::ServiceLifecycleView;
use serde::Serialize;

fn aid(s: &str) -> AllocationId {
    AllocationId::new(s).expect("valid AllocationId")
}

/// The persisted shape of `ServiceLifecycleView` before PROBE-RUN-COUNT:
/// the three reconciler-side streaks and the startup observation stamp.
#[derive(Serialize)]
struct PersistedViewBeforeRunCount {
    startup_attempts_per_alloc: BTreeMap<AllocationId, u32>,
    liveness_consecutive_failures: BTreeMap<(AllocationId, ProbeIdx), u32>,
    readiness_consecutive_successes: BTreeMap<(AllocationId, ProbeIdx), u32>,
    stable_announced: BTreeSet<AllocationId>,
    startup_last_fail_seen_at: BTreeMap<AllocationId, u64>,
    observed: BTreeSet<AllocationId>,
    terminal_announced: BTreeSet<AllocationId>,
}

fn cbor<T: Serialize>(value: &T) -> Vec<u8> {
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).expect("CBOR encode");
    bytes
}

fn decode(bytes: &[u8]) -> ServiceLifecycleView {
    ciborium::from_reader(bytes).expect("CBOR decode of a persisted view")
}

/// A view persisted before PROBE-RUN-COUNT decodes: the removed streak
/// fields are dropped, the announcement sets are kept, and
/// `liveness_last_acted_run` starts empty — so no reconciler-side streak
/// survives the upgrade to be miscounted against the producer's rows.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn persisted_view_with_streak_fields_decodes_without_them() {
    let alloc = aid("svc-0");
    let key = (alloc.clone(), ProbeIdx::new(0));
    let persisted = PersistedViewBeforeRunCount {
        startup_attempts_per_alloc: BTreeMap::from([(alloc.clone(), 4)]),
        liveness_consecutive_failures: BTreeMap::from([(key.clone(), 2)]),
        readiness_consecutive_successes: BTreeMap::from([(key, 1)]),
        stable_announced: BTreeSet::from([alloc.clone()]),
        startup_last_fail_seen_at: BTreeMap::from([(alloc.clone(), 59_000)]),
        observed: BTreeSet::from([alloc.clone()]),
        terminal_announced: BTreeSet::new(),
    };

    let decoded = decode(&cbor(&persisted));

    let expected = ServiceLifecycleView {
        stable_announced: BTreeSet::from([alloc.clone()]),
        observed: BTreeSet::from([alloc]),
        ..ServiceLifecycleView::default()
    };
    assert_eq!(decoded, expected);
    assert!(decoded.liveness_last_acted_run.is_empty());
}

/// The act-once memory survives the runtime's CBOR write-through: a
/// restarted control plane still knows which row each liveness key last
/// terminated on, so a re-read of that row after a restart does not stop the
/// allocation twice.
/// CONTRACT_SHAPE: pure-function.
#[test]
fn liveness_last_acted_run_survives_persistence() {
    let view = ServiceLifecycleView {
        liveness_last_acted_run: BTreeMap::from([
            ((aid("svc-0"), ProbeIdx::new(0)), ProbeRunSeq::new(3).expect("non-zero")),
            ((aid("svc-1"), ProbeIdx::new(0)), ProbeRunSeq::new(7).expect("non-zero")),
        ]),
        ..ServiceLifecycleView::default()
    };

    assert_eq!(decode(&cbor(&view)), view);
}
