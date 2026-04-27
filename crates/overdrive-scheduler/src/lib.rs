//! Pure-function placement scheduler for Overdrive Phase 1.
//!
//! Per ADR-0024 (D4 OVERRIDE), the scheduler lives in its own
//! `core`-class crate so that `dst-lint` mechanically enforces the
//! BTreeMap-only iteration discipline + banned-API contract. The
//! discipline that this file implements:
//!
//! - `schedule(...)` is a pure synchronous function. No `.await`.
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
//! # Status — RED scaffold
//!
//! Phase: phase-1-first-workload, slice 1 (US-01).
//! Wave: DISTILL. The body is `panic!("Not yet implemented -- RED
//! scaffold")` per `.claude/rules/testing.md` § RED scaffolds. The
//! DELIVER crafter implements the first-fit predicate.

#![forbid(unsafe_code)]

/// SCAFFOLD marker — see this file's module docs.
pub const SCAFFOLD: bool = true;

use std::collections::BTreeMap;

use overdrive_core::aggregate::{Job, Node};
use overdrive_core::id::NodeId;
use overdrive_core::traits::driver::Resources;
use overdrive_core::traits::observation_store::AllocStatusRow;

/// First-fit placement decision. Pure synchronous function over
/// deterministic inputs.
///
/// The `BTreeMap` parameter type pins iteration order at the type
/// level — `dst-lint` enforces no `HashMap` appears in this crate's
/// source.
///
/// # Errors
///
/// Returns [`PlacementError::NoCapacity`] when no input node has
/// sufficient free capacity for the job's resource envelope.
/// Returns [`PlacementError::NoHealthyNode`] when the input map is
/// empty.
///
/// # Panics
///
/// Phase: phase-1-first-workload DISTILL. RED scaffold — panics with
/// "Not yet implemented -- RED scaffold". DELIVER fills the body.
#[must_use = "scheduler placement decisions must be acted on"]
pub fn schedule(
    _nodes: &BTreeMap<NodeId, Node>,
    _job: &Job,
    _current_allocs: &[AllocStatusRow],
) -> Result<NodeId, PlacementError> {
    panic!("Not yet implemented -- RED scaffold")
}

/// Placement-failure envelope.
///
/// Phase: phase-1-first-workload DISTILL. The two variants enumerated
/// here are the named cases in US-01 AC; their field shapes are pinned
/// for the proptest (`needed`, `max_free`) and the CLI Pending
/// rendering (`crates/overdrive-control-plane/tests/acceptance/pending_no_capacity_renders_reason.rs`).
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
    /// never produce this; the variant exists for forward-compat
    /// and to allow proptest generators to exercise the boundary.
    #[error("no healthy node in the input set")]
    NoHealthyNode,
}
