//! Pure-function placement scheduler for Overdrive Phase 1.
//!
//! Per ADR-0024 (D4 OVERRIDE), the scheduler lives in its own
//! `core`-class crate so that `dst-lint` mechanically enforces the
//! BTreeMap-only iteration discipline + banned-API contract. The
//! discipline that this file implements:
//!
//! - [`schedule`] is a pure synchronous function. No `.await`.
//! - All iteration runs through `BTreeMap` per
//!   `.claude/rules/development.md` § Ordered-collection choice.
//! - No `Instant::now`, `SystemTime::now`, `rand::*`, or
//!   `tokio::time::sleep` appears in this crate. dst-lint catches
//!   violations at PR time.
//!
//! # Determinism contract
//!
//! For any fixed `(nodes, job, current_allocs)` input, two successive
//! calls return equal `Result<NodeId, PlacementError>`. The proptest
//! in `tests/acceptance/determinism.rs` defends the contract.
//!
//! Iteration of `nodes` is by `BTreeMap`'s `Ord` on `NodeId` —
//! deterministic across `BTreeMap` insertion permutations and across
//! processes / runs / seeds. The acceptance scenario
//! `scheduler_is_invariant_under_btreemap_insertion_order` defends
//! the second half of this contract.
//!
//! # Phase-1 capacity model
//!
//! [`AllocStatusRow`] carries no per-allocation `Resources` field today
//! (REUSE AS-IS per `docs/feature/phase-1-first-workload/design/wave-
//! decisions.md`). The Phase-1 first-fit scheduler therefore treats
//! each `Running` allocation targeting a node as reserving the
//! resource envelope of the *new job being placed*. This is adequate
//! for Phase 1's homogeneous-workload scope; Phase 2+ will add a
//! `resources` field to [`AllocStatusRow`] and switch the scheduler to
//! per-allocation accounting (heterogeneous shape).

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use overdrive_core::aggregate::{Job, Node};
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::{AllocState, AllocStatusRow};

/// First-fit placement decision. Pure synchronous function over
/// deterministic inputs.
///
/// Walks the input `nodes` map in `BTreeMap` order, computes each
/// node's [`free_capacity`] (capacity minus the resources reserved by
/// `Running` allocations targeting it), and returns the [`NodeId`] of
/// the first node whose free capacity meets the job's resource
/// envelope.
///
/// The `BTreeMap` parameter type pins iteration order at the type
/// level — `dst-lint` enforces no `HashMap` appears in this crate's
/// source.
///
/// # Errors
///
/// Returns [`PlacementError::NoCapacity`] when no input node has
/// sufficient free capacity for the job's resource envelope; the error
/// carries `needed` (the job's requested envelope) and `max_free`
/// (the largest free envelope across the input nodes after subtracting
/// running allocations) for actionable diagnostics.
///
/// Returns [`PlacementError::NoHealthyNode`] when the input map is
/// empty.
#[must_use = "scheduler placement decisions must be acted on"]
pub fn schedule(
    nodes: &BTreeMap<NodeId, Node>,
    job: &Job,
    current_allocs: &[AllocStatusRow],
) -> Result<NodeId, PlacementError> {
    // Empty-set guard. Phase-1 single-node never produces this branch
    // operationally; the variant exists so the pure function has a
    // total signature (proptest exercises it via `arb_node_map` lower
    // bound 1, but a future caller could pass an empty map).
    if nodes.is_empty() {
        return Err(PlacementError::NoHealthyNode);
    }

    // Track the largest free envelope seen so far so we can report it
    // back in `NoCapacity::max_free` when no node fits. We start from
    // zero — every iterated node either replaces the running max or
    // keeps it, so the eventual value is the per-component pointwise
    // maximum across nodes after subtraction.
    let mut max_free = Resources { cpu_milli: 0, memory_bytes: 0 };

    // First-fit: walk in BTreeMap order. The first node whose free
    // capacity covers the job's requested envelope wins. The
    // `for (node_id, node) in nodes` form drives BTreeMap's in-order
    // iterator — Ord on NodeId, deterministic across any insertion
    // permutation that yields the same set.
    for (node_id, node) in nodes {
        let free = free_capacity(node, current_allocs, &job.resources);

        if covers(&free, &job.resources) {
            return Ok(node_id.clone());
        }

        // Track the per-component max across nodes for the
        // `NoCapacity::max_free` report. Per-component (rather than
        // dominance-ordered) max means a 2-CPU 0-mem node and a
        // 0-CPU 2-mem node combine into a (2, 2) report, which is
        // less actionable than the per-node max but better matches
        // the operator's "where would I add capacity?" intuition for
        // Phase 1.
        if free.cpu_milli > max_free.cpu_milli {
            max_free.cpu_milli = free.cpu_milli;
        }
        if free.memory_bytes > max_free.memory_bytes {
            max_free.memory_bytes = free.memory_bytes;
        }
    }

    Err(PlacementError::NoCapacity { needed: job.resources, max_free })
}

/// Helper: free capacity of `node` after subtracting the resource
/// envelope reserved by `Running` allocations targeting it.
///
/// Per the Phase-1 capacity model documented at the crate root, each
/// `Running` allocation targeting `node.id` reserves `per_alloc`
/// (the resource envelope of the new job being placed). Subtraction
/// uses [`u32::saturating_sub`] / [`u64::saturating_sub`] to handle
/// the zero-capacity edge without numeric underflow per US-01 AC.
///
/// `Terminated`, `Pending`, `Draining`, and `Suspended` allocations do
/// not reserve capacity — only `Running` does.
#[must_use]
pub fn free_capacity(
    node: &Node,
    current_allocs: &[AllocStatusRow],
    per_alloc: &Resources,
) -> Resources {
    // Count running allocs pinned to this node. Cheap O(N) scan; N is
    // bounded by the cluster's running-alloc count (Phase 1: ≤1).
    let running_on_node: u64 = u64::try_from(
        current_allocs
            .iter()
            .filter(|alloc| alloc.node_id == node.id && alloc.state == AllocState::Running)
            .count(),
    )
    .unwrap_or(u64::MAX);

    // Multiply per-alloc envelope by the count, then subtract from
    // capacity. `saturating_mul` keeps the helper total: an overflow
    // in the multiply (impossible at Phase 1 sizes but cheap insurance)
    // would clamp to MAX, leaving free at 0 after subtraction.
    let total_cpu_reserved = u64::from(per_alloc.cpu_milli).saturating_mul(running_on_node);
    let total_mem_reserved = per_alloc.memory_bytes.saturating_mul(running_on_node);

    // CPU subtraction is widened to u64 to keep the multiply room above,
    // then narrowed to u32. `saturating_sub` cannot exceed
    // `node.capacity.cpu_milli` (a u32) so the narrow is total — but we
    // express it via `u32::try_from(...).unwrap_or(u32::MAX)` to avoid
    // a `cast_possible_truncation` lint.
    let cpu_after = u64::from(node.capacity.cpu_milli).saturating_sub(total_cpu_reserved);
    Resources {
        cpu_milli: u32::try_from(cpu_after).unwrap_or(u32::MAX),
        memory_bytes: node.capacity.memory_bytes.saturating_sub(total_mem_reserved),
    }
}

/// Does `available` cover `needed` on every component?
const fn covers(available: &Resources, needed: &Resources) -> bool {
    available.cpu_milli >= needed.cpu_milli && available.memory_bytes >= needed.memory_bytes
}

/// Placement-failure envelope.
///
/// The two variants enumerated here are the named cases in US-01 AC;
/// their field shapes are pinned for the proptest (`needed`,
/// `max_free`) and the CLI Pending rendering (control-plane Phase 1
/// describe-job acceptance test).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlacementError {
    /// No node has sufficient free capacity for the job's resource
    /// requirements. Carries both the requested envelope and the
    /// largest free envelope across the input nodes for diagnostics.
    #[error("no node has capacity: needed {needed:?}, max free {max_free:?}")]
    NoCapacity {
        /// Resources the job declared.
        needed: Resources,
        /// Largest free envelope across the input nodes after
        /// subtracting running allocations. Reported actionably in
        /// the CLI's Pending row.
        max_free: Resources,
    },
    /// The input `nodes` map is empty. Phase 1 single-node should
    /// never produce this; the variant exists for forward-compat and
    /// to allow proptest generators to exercise the boundary.
    #[error("no healthy node in the input set")]
    NoHealthyNode,
}
