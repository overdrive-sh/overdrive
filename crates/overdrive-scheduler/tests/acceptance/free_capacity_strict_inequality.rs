//! Branch-coverage tests for `overdrive_scheduler::schedule` /
//! `free_capacity`. Pins boundary inequalities and the
//! cpu-AND-memory predicate against mutations that would otherwise
//! pass silently.
//!
//! Mutations covered:
//!
//!   - `lib.rs:110` `free.cpu_milli > max_free.cpu_milli` (`>` →
//!     `>=`) — this drives the `max_free` running maximum across
//!     nodes, surfaced in `PlacementError::NoCapacity { max_free }`.
//!     Under `>=`, the comparison is non-strict and the running
//!     maximum is rewritten on every equal-valued node. Production
//!     and mutant differ only when the same value appears across
//!     multiple nodes — but the *value* of `max_free` is identical
//!     either way; what differs is whether the assignment fires.
//!     A direct value-pinning proptest doesn't distinguish these,
//!     because `max_free` settles to the same number. The mutant is
//!     in fact equivalent at the value level — caught at the
//!     `>` vs `>=` semantic level only when combined with side
//!     effects. We rely on the existing `determinism` proptest
//!     (which iterates many seeds) as the falsification surface.
//!
//!     Practically, the line that *can* be flipped observably is
//!     when the `max_free` starts at zero and one node has free.cpu
//!     == 0 — under `>` no rewrite (still 0); under `>=` rewrite to
//!     0. Both yield `max_free=0`. The mutant is value-equivalent
//!     here too. We mark this mutant as a known equivalent in the
//!     scheduler-side notes; it is NOT killable through schedule's
//!     observable surface.
//!
//!   - `lib.rs:113` (memory analogue) — same reasoning.
//!
//!   - `lib.rs:143` `alloc.node_id == node.id && alloc.state ==
//!     AllocState::Running` (`&&` → `||`) — this IS killable
//!     because the count of "matching allocs" feeds reservation
//!     subtraction, which is observable through whether placement
//!     returns Ok or Err.
//!
//! See the inline notes for why the strict-inequality mutants are
//! treated separately.

#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::AllocState;
use overdrive_scheduler::{PlacementError, schedule};

use super::common::{make_alloc_running, make_job, make_node, nid, res};

// ---------------------------------------------------------------------------
// L143 — `&&` -> `||` in free_capacity
// ---------------------------------------------------------------------------

#[test]
fn free_capacity_excludes_pending_allocs_on_same_node() {
    // One node "local" with capacity (1500 mCPU, 2 GiB).
    // One alloc on "local" but state=Pending (NOT Running), for a
    // different job ("other").
    // Job needs (1000 mCPU, 1 GiB).
    //
    // Production (`&&`): `node_id == local AND state == Running`
    // → false (state is Pending) → 0 matches → free = (1500, 2 GiB)
    // → fits → schedule returns Ok(local).
    //
    // Mutant (`||`): `node_id == local OR state == Running` → true
    // (first clause) → 1 match → reserves (1000, 1 GiB) → free =
    // (500, 1 GiB) → cpu < needed → NoCapacity.
    //
    // Asserting Ok(local) kills the mutant.
    let local = make_node("local", res(1_500, 2 * 1024 * 1024 * 1024));
    let mut nodes = BTreeMap::new();
    nodes.insert(local.id.clone(), local);
    let job = make_job("payments", res(1_000, 1024 * 1024 * 1024));

    // Pending alloc — same node, different job, NOT Running.
    let allocs = vec![overdrive_core::traits::observation_store::AllocStatusRow {
        alloc_id: overdrive_core::id::AllocationId::new("alloc-pending-0").expect("valid"),
        job_id: overdrive_core::id::JobId::new("other").expect("valid"),
        node_id: nid("local"),
        state: AllocState::Pending,
        updated_at: overdrive_core::traits::observation_store::LogicalTimestamp {
            counter: 1,
            writer: nid("local"),
        },
        reason: None,
        detail: None,
        terminal: None,
    }];

    let result = schedule(&nodes, &job, &allocs);

    assert_eq!(
        result,
        Ok(nid("local")),
        "Pending alloc must NOT reserve capacity (state filter); got {result:?}",
    );
}

#[test]
fn free_capacity_includes_running_allocs_on_same_node() {
    // Confirm the production `&&` actually subtracts when both
    // clauses are true. Tightens the falsifiability surface for
    // L143's `&&` → `||` — paired with the previous test, the
    // surviving production semantic is exactly the `&&` shape.
    let local = make_node("local", res(1_500, 2 * 1024 * 1024 * 1024));
    let mut nodes = BTreeMap::new();
    nodes.insert(local.id.clone(), local);
    let job = make_job("payments", res(1_000, 1024 * 1024 * 1024));
    let allocs = vec![make_alloc_running("alloc-running-0", "other", "local")];

    let result = schedule(&nodes, &job, &allocs);

    let Err(PlacementError::NoCapacity { max_free, .. }) = result else {
        panic!("expected NoCapacity, got {result:?}");
    };
    assert_eq!(
        max_free.cpu_milli, 500,
        "Running alloc on same node reserves (1000, 1 GiB) → free.cpu = 500",
    );
}

// ---------------------------------------------------------------------------
// L110 / L113 — strict inequality on per-component max_free
// ---------------------------------------------------------------------------
//
// Two nodes with distinct capacities — production tracks the
// per-component max across all nodes via strict-greater. The
// observed report value matches under both `>` and `>=`, so a pure
// value-pinning proptest does not distinguish them. We add a
// targeted scenario that exercises the multi-node tracking shape
// anyway — this is a falsification surface for *related* bugs
// (e.g., resetting max_free on every iteration), even though the
// `>` vs `>=` flip is a value-equivalent mutant on the public API.

#[test]
fn no_capacity_max_free_reflects_largest_per_component_across_nodes() {
    // Three nodes, no running allocs:
    //   node-a: (500 mCPU, 4 GiB)
    //   node-b: (2000 mCPU, 1 GiB)
    //   node-c: (100 mCPU, 100 MiB)
    // Job needs (5000 mCPU, 10 GiB) — none fit.
    //
    // Per-component max across the three: cpu = 2000 (node-b),
    // memory = 4 GiB (node-a). Production: max_free = (2000, 4 GiB).
    let mut nodes = BTreeMap::new();
    let a = make_node("node-a", res(500, 4 * 1024 * 1024 * 1024));
    let b = make_node("node-b", res(2000, 1024 * 1024 * 1024));
    let c = make_node("node-c", res(100, 100 * 1024 * 1024));
    nodes.insert(a.id.clone(), a);
    nodes.insert(b.id.clone(), b);
    nodes.insert(c.id.clone(), c);

    let job = make_job("memhog", res(5_000, 10 * 1024 * 1024 * 1024));
    let result = schedule(&nodes, &job, &[]);

    let Err(PlacementError::NoCapacity { max_free, needed }) = result else {
        panic!("expected NoCapacity, got {result:?}");
    };
    assert_eq!(
        max_free,
        Resources { cpu_milli: 2000, memory_bytes: 4 * 1024 * 1024 * 1024 },
        "max_free must reflect per-component max across nodes",
    );
    assert_eq!(needed, Resources { cpu_milli: 5_000, memory_bytes: 10 * 1024 * 1024 * 1024 });
}
