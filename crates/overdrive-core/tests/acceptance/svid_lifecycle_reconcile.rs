//! Acceptance tests for `SvidLifecycle::reconcile` per ADR-0067 D1/D2
//! (workload-identity-manager Slice 01, step 01-04).
//!
//! Layer 1: pure reconciler scenarios. The reconciler converges
//! `desired = running allocs` against `actual = the IdentityMgr held set`
//! and emits the diff (`running ∧ ¬held → IssueSvid`,
//! `¬running ∧ held → DropSvid`). `reconcile` is a pure function — no
//! `.await`, no CA / `ObservationStore` handle, wall-clock only via
//! `tick.now` — so these tests construct the typed `State` directly and
//! assert on the emitted action list (the observable universe) without any
//! infrastructure double.
//!
//! Lives in `tests/acceptance/` rather than `src/` because dst-lint scans
//! `src/**/*.rs` and bans `Instant::now()` there even under `#[cfg(test)]`;
//! the `TickContext.now` snapshot the reconcile signature requires forces
//! the `Instant` read into the test fixture here.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use overdrive_core::id::{AllocationId, ContentHash, CorrelationKey, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::svid_lifecycle::{
    RunningAlloc, SvidLifecycle, SvidLifecycleState,
};
use overdrive_core::reconcilers::{Action, HeldSvidFacts, Reconciler, TickContext};
use overdrive_core::wall_clock::UnixInstant;
use proptest::prelude::*;

const NODE_RAW: &str = "local";

/// RED scaffold marker for the Slice-03 / Slice-01-enqueue scenarios
/// (S-WIM-08 retry-memory View, S-WIM-09 emit-gated rotation seam, S-WIM-10
/// lifecycle enqueue) that land in later steps (03-01 / 03-02 / 01-05), not
/// 01-04. Kept as `#[should_panic(expected = "RED scaffold")]` per
/// `.claude/rules/testing.md`.
fn red_scaffold(scenario: &str) -> ! {
    panic!("RED scaffold: workload-identity-manager {scenario}");
}

fn make_tick(now_secs: u64) -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(now_secs)),
        tick: now_secs,
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

fn workload() -> WorkloadId {
    WorkloadId::new("payments").expect("valid WorkloadId")
}

fn node() -> NodeId {
    NodeId::new(NODE_RAW).expect("valid NodeId")
}

fn running_alloc() -> RunningAlloc {
    RunningAlloc { workload_id: workload(), node_id: node() }
}

fn held_facts(alloc: &AllocationId, not_after_secs: u64) -> HeldSvidFacts {
    HeldSvidFacts {
        spiffe_id: SpiffeId::for_allocation(&workload(), alloc),
        not_after: UnixInstant::from_unix_duration(Duration::from_secs(not_after_secs)),
    }
}

/// The deterministic `IssueSvid` correlation the reconciler must derive for a
/// given allocation (ADR-0067 D2): `target = "svid-lifecycle/<alloc>"`,
/// `spec_hash = ContentHash::of(<spiffe-uri bytes>)`, `purpose = "issue-svid"`.
fn expected_issue_correlation(alloc: &AllocationId, spiffe: &SpiffeId) -> CorrelationKey {
    let target = format!("svid-lifecycle/{alloc}");
    let spec_hash = ContentHash::of(spiffe.as_str().as_bytes());
    CorrelationKey::derive(&target, &spec_hash, "issue-svid")
}

/// The deterministic `DropSvid` correlation (purpose `"drop-svid"`); the
/// dropped allocation's `spec_hash` is derived from the held identity.
fn expected_drop_correlation(alloc: &AllocationId, spiffe: &SpiffeId) -> CorrelationKey {
    let target = format!("svid-lifecycle/{alloc}");
    let spec_hash = ContentHash::of(spiffe.as_str().as_bytes());
    CorrelationKey::derive(&target, &spec_hash, "drop-svid")
}

// `@in-memory` `@property` `@S-WIM-01` — for an arbitrary running-set / held-set
// population, EVERY allocation that is Running but NOT held yields exactly one
// `Action::IssueSvid` carrying the four pinned fields (`alloc_id`, the
// `SpiffeId::for_allocation` identity, `node_id`, the derived correlation), and
// the reconcile body performs no CA / ObservationStore I/O (it is a pure
// function over its arguments — there is no handle to call). The observable
// universe is the emitted action list + the returned `next_view`.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    #[test]
    fn running_alloc_without_held_svid_emits_issue_svid(
        running_suffixes in proptest::collection::btree_set("[a-f0-9]{4,8}", 1..6),
        held_extra_suffixes in proptest::collection::btree_set("[a-f0-9]{4,8}", 0..4),
    ) {
        let reconciler = SvidLifecycle::canonical();

        // desired = the running set.
        let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
        for suffix in &running_suffixes {
            let alloc = AllocationId::new(&format!("payments-{suffix}")).expect("valid AllocationId");
            desired.insert(alloc.clone(), running_alloc());
        }

        // actual = the held set: hold SVIDs for a DISJOINT extra set only, so
        // every running alloc is `running ∧ ¬held`. (Suffixes that collide with
        // the running set are filtered out to keep the property's precondition
        // — "running without held" — true for every running alloc.)
        let mut actual: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
        for suffix in held_extra_suffixes.iter().filter(|s| !running_suffixes.contains(*s)) {
            let alloc = AllocationId::new(&format!("payments-{suffix}")).expect("valid AllocationId");
            actual.insert(alloc.clone(), held_facts(&alloc, 1_700_000_000));
        }

        let state = SvidLifecycleState { desired: desired.clone(), actual: actual.clone() };
        let view = <SvidLifecycle as Reconciler>::View::default();
        let tick = make_tick(0);

        let (actions, next_view) = reconciler.reconcile(&state, &state, &view, &tick);

        // Universe slot 1: the emitted IssueSvid set.
        let issues: BTreeMap<AllocationId, Action> = actions
            .iter()
            .filter(|a| matches!(a, Action::IssueSvid { .. }))
            .map(|a| match a {
                Action::IssueSvid { alloc_id, .. } => (alloc_id.clone(), a.clone()),
                _ => unreachable!(),
            })
            .collect();

        // Exactly one IssueSvid per running-without-held alloc — no more, no less.
        prop_assert_eq!(
            issues.len(),
            running_suffixes.len(),
            "one IssueSvid per running ∧ ¬held alloc"
        );

        for alloc in desired.keys() {
            let expected_spiffe = SpiffeId::for_allocation(&workload(), alloc);
            let action = issues.get(alloc).expect("running ∧ ¬held alloc emits an IssueSvid");
            match action {
                Action::IssueSvid { alloc_id, spiffe_id, node_id, correlation } => {
                    prop_assert_eq!(alloc_id, alloc, "IssueSvid names the running alloc");
                    prop_assert_eq!(
                        spiffe_id,
                        &expected_spiffe,
                        "IssueSvid carries SpiffeId::for_allocation"
                    );
                    prop_assert_eq!(node_id, &node(), "IssueSvid carries the issuing node");
                    prop_assert_eq!(
                        correlation,
                        &expected_issue_correlation(alloc, &expected_spiffe),
                        "IssueSvid correlation derives from (svid-lifecycle/<alloc>, spec_hash, issue-svid)"
                    );
                }
                _ => unreachable!(),
            }
        }

        // No held alloc that is also running was dropped, and no extra-held
        // alloc was issued (the extra held set is disjoint from running, so it
        // is `¬running ∧ held → DropSvid`, never IssueSvid).
        for alloc in actual.keys() {
            prop_assert!(
                !issues.contains_key(alloc),
                "a held (not-running) alloc is never issued"
            );
        }

        // Universe slot 2: the next_view is the minimal Slice-01 view
        // (retry memory lands in 03-01) — unchanged default.
        prop_assert_eq!(
            next_view,
            <SvidLifecycle as Reconciler>::View::default(),
            "Slice-01 View is minimal: reconcile returns the default unchanged"
        );
    }
}

/// `@in-memory` `@S-WIM-03` — a stopped (no-longer-Running) allocation that still
/// has a held SVID yields exactly one `Action::DropSvid` for it (so the leaf key
/// becomes unreachable, ADR-0067 O2), and emits NO `IssueSvid` for the stopped
/// alloc. The still-running held alloc converges to a no-op.
#[test]
fn stopped_alloc_with_held_svid_emits_drop_svid() {
    let reconciler = SvidLifecycle::canonical();

    let stopped = AllocationId::new("payments-dead").expect("valid AllocationId");
    let still_running = AllocationId::new("payments-live").expect("valid AllocationId");

    // desired = only the still-running alloc.
    let mut desired: BTreeMap<AllocationId, RunningAlloc> = BTreeMap::new();
    desired.insert(still_running.clone(), running_alloc());

    // actual = both allocs are held (the stopped one is a leftover hold).
    let mut actual: BTreeMap<AllocationId, HeldSvidFacts> = BTreeMap::new();
    actual.insert(stopped.clone(), held_facts(&stopped, 1_700_000_000));
    actual.insert(still_running.clone(), held_facts(&still_running, 1_700_000_000));

    let state = SvidLifecycleState { desired, actual };
    let view = <SvidLifecycle as Reconciler>::View::default();
    let tick = make_tick(0);

    let (actions, _next_view) = reconciler.reconcile(&state, &state, &view, &tick);

    // Exactly one DropSvid, and it targets the stopped alloc.
    let drops: Vec<&Action> =
        actions.iter().filter(|a| matches!(a, Action::DropSvid { .. })).collect();
    assert_eq!(drops.len(), 1, "exactly one DropSvid for the ¬running ∧ held alloc");

    let stopped_spiffe = SpiffeId::for_allocation(&workload(), &stopped);
    match drops[0] {
        Action::DropSvid { alloc_id, correlation } => {
            assert_eq!(alloc_id, &stopped, "DropSvid names the stopped alloc");
            assert_eq!(
                correlation,
                &expected_drop_correlation(&stopped, &stopped_spiffe),
                "DropSvid correlation derives from (svid-lifecycle/<alloc>, spec_hash, drop-svid)"
            );
        }
        _ => unreachable!(),
    }

    // No IssueSvid for the stopped alloc, and the still-running held alloc is a
    // no-op (neither issued nor dropped).
    for action in &actions {
        match action {
            Action::IssueSvid { alloc_id, .. } => {
                panic!(
                    "no IssueSvid expected (stopped is dropped, live is already held): {alloc_id}"
                )
            }
            Action::DropSvid { alloc_id, .. } => {
                assert_ne!(
                    alloc_id, &still_running,
                    "the still-running held alloc is never dropped"
                );
            }
            _ => {}
        }
    }
}

/// `@in-memory` `@property` `@S-WIM-08` -- the View is retry memory only:
/// `IssueRetry { attempts, last_failure_seen_at }`, with no serial,
/// `issued_at`, `spiffe_id`, `expires_at`, or `next_renewal_at` success fact.
#[test]
#[should_panic(expected = "RED scaffold")]
fn svid_lifecycle_view_is_retry_memory_only() {
    red_scaffold("S-WIM-08 View is retry memory only");
}

/// `@in-memory` `@error` `@S-WIM-09` -- the #40 near-expiry branch is
/// structurally present but emit-gated until `cert_rotation` is registered,
/// so #35 never emits `UnknownWorkflow` every tick.
#[test]
#[should_panic(expected = "RED scaffold")]
fn near_expiry_rotation_seam_is_emit_gated_until_cert_rotation_registered() {
    red_scaffold("S-WIM-09 rotation seam is emit-gated");
}

/// `@in-memory` `@S-WIM-10` -- `WorkloadLifecycle` and the exit observer enqueue
/// `SvidLifecycle` evaluation on alloc Running/Stopped transitions. Without this,
/// the pure reconciler can be correct but unreachable.
#[test]
#[should_panic(expected = "RED scaffold")]
fn workload_lifecycle_transitions_enqueue_svid_lifecycle() {
    red_scaffold("S-WIM-10 lifecycle transitions enqueue SvidLifecycle");
}
