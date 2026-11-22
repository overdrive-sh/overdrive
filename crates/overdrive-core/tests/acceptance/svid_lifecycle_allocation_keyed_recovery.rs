//! `SvidLifecycle` keys restart recovery and held-set attribution on the
//! `AllocationId`, not on the identity (workload-spiffe-identity D2 / DD-3,
//! ADR-0345). Two allocations of one workload share one SPIFFE identity, so an
//! identity-keyed `ever_issued` cannot tell them apart; an `AllocationId`-keyed
//! one can.
//!
//! The sibling `svid_lifecycle_reconcile.rs` and `svid_lifecycle_rotation.rs`
//! cover the single-allocation recovery and rotation scenarios over the same
//! `ever_issued: BTreeSet<AllocationId>` shape.
//!
//! Layer: unit, pure `reconcile` over typed state (no I/O, no clock handle).
//! CONTRACT_SHAPE: pure-function.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::identity::HeldSvidFacts;
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::svid_lifecycle::{
    IssueRetry, RunningAlloc, SvidLifecycle, SvidLifecycleState, SvidLifecycleView,
};

const NOW_SECS: u64 = 1_700_000_000;

fn workload() -> WorkloadId {
    WorkloadId::new("payments").expect("valid WorkloadId")
}

fn alloc(raw: &str) -> AllocationId {
    AllocationId::new(raw).expect("valid AllocationId")
}

fn running_alloc() -> RunningAlloc {
    RunningAlloc { workload_id: workload(), node_id: NodeId::new("local").expect("valid NodeId") }
}

fn tick() -> TickContext {
    TickContext {
        now: Instant::now(),
        now_unix: UnixInstant::from_unix_duration(Duration::from_secs(NOW_SECS)),
        tick: NOW_SECS,
        deadline: Instant::now() + Duration::from_secs(60),
    }
}

fn issued(actions: &[Action]) -> BTreeMap<AllocationId, SpiffeId> {
    actions
        .iter()
        .filter_map(|action| match action {
            Action::IssueSvid { alloc_id, spiffe_id, .. } => {
                Some((alloc_id.clone(), spiffe_id.clone()))
            }
            _ => None,
        })
        .collect()
}

/// Allocation A was issued an SVID before the restart (`ever_issued`), its
/// sibling B of the SAME workload was not. Both are Running, unheld, and carry a
/// just-failed retry entry. A's recovery re-issues immediately (it bypasses the
/// backoff gate); B is first-issue and stays inside its backoff window. With an
/// identity-keyed `ever_issued` the shared identity would make B look recovered
/// too.
#[test]
fn restart_recovery_applies_to_the_allocation_that_was_issued_not_its_sibling() {
    let (a, b) = (alloc("payments-0"), alloc("payments-1"));
    let desired = BTreeMap::from([(a.clone(), running_alloc()), (b.clone(), running_alloc())]);
    let state = SvidLifecycleState {
        desired,
        actual: BTreeMap::new(),
        ever_issued: BTreeSet::from([a.clone()]),
    };
    let just_failed = IssueRetry { attempts: 1, last_failure_seen_at: tick().now_unix };
    let mut view = SvidLifecycleView::default();
    view.retry.insert(a.clone(), just_failed.clone());
    view.retry.insert(b.clone(), just_failed);

    let (actions, _) = SvidLifecycle::canonical().reconcile(&state, &state, &view, &tick());

    let issued = issued(&actions);
    assert!(issued.contains_key(&a), "the issued allocation recovers immediately: {actions:?}");
    assert!(
        !issued.contains_key(&b),
        "its sibling did not inherit ever_issued and stays backed off: {actions:?}"
    );
}

/// Held-set attribution stays by the `AllocationId` that keys the held set: A
/// holds an SVID whose identity is the workload identity; its sibling B holds
/// none. Exactly B is issued; A is not dropped or re-issued.
#[test]
fn held_attribution_is_per_allocation_when_two_allocations_share_the_identity() {
    let (a, b) = (alloc("payments-0"), alloc("payments-1"));
    let desired = BTreeMap::from([(a.clone(), running_alloc()), (b.clone(), running_alloc())]);
    let actual = BTreeMap::from([(
        a.clone(),
        HeldSvidFacts {
            spiffe_id: SpiffeId::for_workload(&workload()),
            not_after: UnixInstant::from_unix_duration(Duration::from_secs(NOW_SECS + 3_600)),
        },
    )]);
    let state = SvidLifecycleState { desired, actual, ever_issued: BTreeSet::from([a]) };

    let (actions, _) = SvidLifecycle::canonical().reconcile(
        &state,
        &state,
        &SvidLifecycleView::default(),
        &tick(),
    );

    assert_eq!(issued(&actions).keys().cloned().collect::<Vec<_>>(), vec![b], "{actions:?}");
    assert!(
        !actions.iter().any(|action| matches!(action, Action::DropSvid { .. })),
        "the held allocation is not dropped: {actions:?}"
    );
}
