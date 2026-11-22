//! A workload's SPIFFE identity names the workload, not the allocation
//! (workload-spiffe-identity, D1 / DD-1, ADR-0344).
//!
//! `SpiffeId::for_workload(&WorkloadId)` builds
//! `spiffe://overdrive.local/workload/<w>`; `SpiffeId::workload_id()` is its
//! one inverse and the parse SSOT (the three private parsers route through it).
//!
//! Layer: unit, port-to-port at domain scope (both are pure functions of their
//! public signature).
//!
//! CONTRACT_SHAPE: pure-function.

#![allow(clippy::expect_used, clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use overdrive_core::id::{AllocationId, NodeId, SpiffeId, WorkloadId};
use overdrive_core::reconcilers::{Action, Reconciler, TickContext};
use overdrive_core::wall_clock::UnixInstant;
use overdrive_reconcilers::svid_lifecycle::{
    RunningAlloc, SvidLifecycle, SvidLifecycleState, SvidLifecycleView,
};
use proptest::prelude::*;

/// A valid workload label: DNS-1123-label-like, lowercase ASCII alphanumerics
/// plus `-` / `_` / `.`, leading and trailing alphanumeric.
fn workload_labels() -> impl Strategy<Value = WorkloadId> {
    "[a-z0-9]([a-z0-9_.-]{0,40}[a-z0-9])?"
        .prop_filter_map("a valid WorkloadId", |label| WorkloadId::new(&label).ok())
}

fn allocation_labels() -> impl Strategy<Value = AllocationId> {
    "[a-z0-9]([a-z0-9_.-]{0,40}[a-z0-9])?"
        .prop_filter_map("a valid AllocationId", |label| AllocationId::new(&label).ok())
}

fn spiffe(raw: &str) -> SpiffeId {
    SpiffeId::new(raw).expect("a syntactically valid SPIFFE ID")
}

proptest! {
    /// `for_workload(w).workload_id() == Some(w)` for every valid workload, and
    /// the identity is exactly `spiffe://overdrive.local/workload/<w>`.
    #[test]
    fn a_workload_identity_round_trips_to_its_workload(workload in workload_labels()) {
        let identity = SpiffeId::for_workload(&workload);
        prop_assert_eq!(
            identity.as_str(),
            format!("spiffe://overdrive.local/workload/{}", workload.as_str())
        );
        prop_assert_eq!(identity.workload_id(), Some(workload));
    }

    /// The superseded per-allocation shape `/workload/<w>/alloc/<a>` is not a
    /// workload identity: `workload_id()` is `None`, there is no parser of the
    /// old shape (DD-5: greenfield single cut).
    #[test]
    fn the_superseded_allocation_shaped_identity_names_no_workload(
        workload in workload_labels(),
        alloc in allocation_labels(),
    ) {
        let old = spiffe(&format!(
            "spiffe://overdrive.local/workload/{}/alloc/{}",
            workload.as_str(),
            alloc.as_str()
        ));
        prop_assert_eq!(old.workload_id(), None);
    }
}

/// The concrete wire shape: workload `payments` is
/// `spiffe://overdrive.local/workload/payments`, and the identity names it back.
#[test]
fn the_payments_workload_has_the_documented_identity() {
    let workload = WorkloadId::new("payments").expect("valid workload");
    let identity = SpiffeId::for_workload(&workload);
    assert_eq!(identity.as_str(), "spiffe://overdrive.local/workload/payments");
    assert_eq!(identity.workload_id(), Some(workload));
}

/// Every shape that is not exactly `/workload/<valid label>` names no workload:
/// the gateway identity, an empty or invalid label, extra segments, other path
/// families.
#[test]
fn identities_that_are_not_exactly_a_workload_path_name_no_workload() {
    // Build every identity first: a rejection by `SpiffeId::new` fails here, on
    // the input, not inside the `workload_id` assertion.
    let identities: Vec<(&str, SpiffeId)> = [
        "spiffe://overdrive.local/gateway/node-1",
        "spiffe://overdrive.local/workload",
        "spiffe://overdrive.local/workload/",
        "spiffe://overdrive.local/workload/-leading-hyphen",
        "spiffe://overdrive.local/workload/trailing-hyphen-",
        "spiffe://overdrive.local/workload/payments/",
        "spiffe://overdrive.local/workload/payments/extra",
        "spiffe://overdrive.local/node/node-1/workload/payments",
        "spiffe://overdrive.local/ns/default/sa/payments",
    ]
    .into_iter()
    .map(|raw| (raw, spiffe(raw)))
    .collect();
    for (raw, identity) in identities {
        assert_eq!(identity.workload_id(), None, "{raw} names no workload");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Every Running, unheld allocation of one workload is issued the WORKLOAD
    /// identity (shared by all of them), each under its own allocation
    /// (D1 / D2, ADR-0344, ADR-0345): one `IssueSvid` per allocation, all naming
    /// `spiffe://overdrive.local/workload/payments`.
    ///
    /// CONTRACT_SHAPE: pure-function.
    #[test]
    fn every_allocation_of_a_workload_is_issued_the_workload_identity(
        suffixes in proptest::collection::btree_set("[a-f0-9]{4,8}", 1..6),
    ) {
        let workload = WorkloadId::new("payments").expect("valid WorkloadId");
        let running = RunningAlloc {
            workload_id: workload,
            node_id: NodeId::new("local").expect("valid NodeId"),
        };
        let desired: BTreeMap<AllocationId, RunningAlloc> = suffixes
            .iter()
            .map(|suffix| {
                let alloc = AllocationId::new(&format!("payments-{suffix}"))
                    .expect("valid AllocationId");
                (alloc, running.clone())
            })
            .collect();
        let state = SvidLifecycleState {
            desired: desired.clone(),
            actual: BTreeMap::new(),
            ever_issued: BTreeSet::new(),
        };
        let tick = TickContext {
            now: Instant::now(),
            now_unix: UnixInstant::from_unix_duration(Duration::from_secs(1_700_000_000)),
            tick: 1,
            deadline: Instant::now() + Duration::from_secs(60),
        };

        let (actions, _) =
            SvidLifecycle::canonical().reconcile(&state, &state, &SvidLifecycleView::default(), &tick);

        let issued: BTreeMap<AllocationId, SpiffeId> = actions
            .iter()
            .filter_map(|action| match action {
                Action::IssueSvid { alloc_id, spiffe_id, .. } => {
                    Some((alloc_id.clone(), spiffe_id.clone()))
                }
                _ => None,
            })
            .collect();
        prop_assert_eq!(
            issued.keys().cloned().collect::<BTreeSet<_>>(),
            desired.keys().cloned().collect::<BTreeSet<_>>()
        );
        for identity in issued.values() {
            prop_assert_eq!(
                identity.as_str(),
                "spiffe://overdrive.local/workload/payments",
                "every allocation is issued its workload's identity"
            );
        }
    }
}
