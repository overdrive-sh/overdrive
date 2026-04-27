//! US-01 Scenarios 1.2, 1.3 — Scheduler determinism + BTreeMap-order invariance.
//!
//! Determinism is the **K1** KPI for slice 1: `schedule(...)` must be a
//! pure function over its inputs — same `(nodes, job, current_allocs)`
//! input MUST produce identical `Result<NodeId, PlacementError>`.
//!
//! 1.2 covers raw input-equality determinism: build one input, call
//! `schedule` twice, assert both calls return equal `Result`.
//!
//! 1.3 covers `BTreeMap`-insertion-order invariance: build a *set* of
//! nodes, construct two `BTreeMap`s from the same set under different
//! insertion orders, assert both produce equal `Result`. This pins
//! the §18 reconciler-runtime contract that hot-path iteration is
//! through `BTreeMap` (no `RandomState` smuggling per
//! `.claude/rules/development.md` § Ordered-collection choice).

use std::collections::BTreeMap;

use proptest::prelude::*;

use overdrive_core::id::NodeId;
use overdrive_scheduler::schedule;

use super::common::{arb_allocs_for_nodes, arb_job, arb_node_map};

proptest! {
    /// Scenario 1.2 — calling `schedule` twice on the same input
    /// returns the same `Result`. K1 KPI defended.
    #[test]
    fn scheduler_is_deterministic_under_proptest(
        (nodes, job, allocs) in arb_node_map().prop_flat_map(|nodes| {
            let node_ids: Vec<NodeId> = nodes.keys().cloned().collect();
            let allocs = arb_allocs_for_nodes(node_ids);
            (Just(nodes), arb_job(), allocs)
        })
    ) {
        let first = schedule(&nodes, &job, &allocs);
        let second = schedule(&nodes, &job, &allocs);
        prop_assert_eq!(
            first,
            second,
            "schedule must be deterministic for identical inputs"
        );
    }
}

proptest! {
    /// Scenario 1.3 — constructing the same node *set* in two different
    /// insertion orders produces equal `Result`. Defends against any
    /// future `HashMap` smuggling: a Hash-keyed map's iteration order
    /// is process-randomized, so a hash-routed scheduler would fail
    /// this test on the second call.
    #[test]
    fn scheduler_is_invariant_under_btreemap_insertion_order(
        (nodes_orig, job, allocs) in arb_node_map().prop_flat_map(|nodes| {
            let node_ids: Vec<NodeId> = nodes.keys().cloned().collect();
            let allocs = arb_allocs_for_nodes(node_ids);
            (Just(nodes), arb_job(), allocs)
        })
    ) {
        // Build a second BTreeMap by reinserting the entries in reverse
        // order. If the scheduler is BTreeMap-honest, both maps drive
        // the same iteration sequence and the result is equal.
        let entries: Vec<_> = nodes_orig.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let mut nodes_reversed: BTreeMap<NodeId, _> = BTreeMap::new();
        for (k, v) in entries.into_iter().rev() {
            nodes_reversed.insert(k, v);
        }

        let r_orig = schedule(&nodes_orig, &job, &allocs);
        let r_rev = schedule(&nodes_reversed, &job, &allocs);
        prop_assert_eq!(
            r_orig,
            r_rev,
            "schedule must be invariant under BTreeMap insertion order"
        );
    }
}
