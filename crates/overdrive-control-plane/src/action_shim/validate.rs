//! Reconcile-output invariant validator.
//!
//! Runtime defense against a buggy reconciler that emits a
//! `ReclaimAllocationNetwork` beside another action naming the SAME
//! allocation in a single `reconcile()` return.
//!
//! # Why this lives at the dispatch boundary
//!
//! The convergence loop dispatches actions sequentially through the
//! [`action_shim::dispatch`](super::dispatch) loop. Sum-type-interior
//! modelling on the [`Action`] enum is insufficient — the enum admits valid
//! actions whose Vec-level composition is a bug. The runtime validator is
//! the right layer: it inspects the post-`reconcile` Vec and rejects the
//! aggregate before any dispatch fires.
//!
//! # Fail-safe semantics
//!
//! On violation, the caller [`run_convergence_tick`](crate::reconciler_runtime::run_convergence_tick)
//! skips action dispatch for the tick and emits a structured
//! `reconciler.output.invariant_violation` tracing event. The View still
//! persists (reconciler memory is independent of dispatch success);
//! convergence retries the next tick. The control-plane does NOT panic on a
//! buggy reconciler.
//!
//! Per `.claude/rules/development.md` § "Distinct failure modes get
//! distinct error variants": the validator returns a typed
//! [`ConflictingAllocationReclaim`] with named structural fields so
//! downstream branches do not have to parse `Display` strings.
//!
//! Per `.claude/rules/development.md` § "Ordered-collection choice": the
//! tracking sets are ordered collections so the FIRST conflicting pair
//! surfaced does not depend on hash iteration order.

use std::collections::{BTreeMap, BTreeSet};

use overdrive_core::id::AllocationId;
use overdrive_core::reconcilers::Action;

/// The allocation action a `ReclaimAllocationNetwork` conflicted with in one
/// `reconcile()` return: the other action names the same `alloc_id`.
///
/// GH #295 D-295-R11 (FD § "[REF] Lifecycle action — row-neutral reclaim (D-295-R11) — ACCEPTED 2026-09-24" (the reclaim action, shim arm, and validator rule)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReclaimConflictAction {
    /// `Action::StartAllocation` for the same `alloc_id`.
    StartAllocation,
    /// `Action::RestartAllocation` whose `alloc_id` (the predecessor) is the
    /// same allocation.
    RestartAllocation,
    /// `Action::StopAllocation` for the same `alloc_id`.
    StopAllocation,
    /// `Action::FinalizeFailed` for the same `alloc_id`.
    FinalizeFailed,
}

/// Violation surfaced by [`validate_reconcile_output`]: a
/// `ReclaimAllocationNetwork` and another allocation action name the same
/// allocation in one `reconcile()` return (GH #295 D-295-R11). Per
/// `.claude/rules/development.md` § Errors: typed structural fields, not a
/// flat string.
#[derive(Debug, Clone, thiserror::Error)]
#[error("conflicting allocation reclaim at alloc={alloc_id}: {other:?} names the same allocation")]
pub struct ConflictingAllocationReclaim {
    /// The allocation both actions name.
    pub alloc_id: AllocationId,
    /// The action that names it beside the reclaim.
    pub other: ReclaimConflictAction,
}

// Returns the action's allocation key and conflict category for the R11 pair rule.
fn reclaim_owner(action: &Action) -> Option<(&AllocationId, ReclaimConflictAction)> {
    match action {
        Action::StartAllocation { alloc_id, .. } => {
            Some((alloc_id, ReclaimConflictAction::StartAllocation))
        }
        Action::RestartAllocation { alloc_id, .. } => {
            Some((alloc_id, ReclaimConflictAction::RestartAllocation))
        }
        Action::StopAllocation { alloc_id, .. } => {
            Some((alloc_id, ReclaimConflictAction::StopAllocation))
        }
        Action::FinalizeFailed { alloc_id, .. } => {
            Some((alloc_id, ReclaimConflictAction::FinalizeFailed))
        }
        _ => None,
    }
}

/// Walk `actions` in emission order; return `Err` on the first
/// inter-action conflict. The contract is *first-conflict-wins* — the
/// dispatch boundary's response is the same regardless (skip dispatch this
/// tick, log, retry next tick).
///
/// # Errors
///
/// Returns [`ConflictingAllocationReclaim`] when a
/// `ReclaimAllocationNetwork` and one of `StartAllocation`,
/// `RestartAllocation`, `StopAllocation`, or `FinalizeFailed` name the
/// same allocation.
pub fn validate_reconcile_output(actions: &[Action]) -> Result<(), ConflictingAllocationReclaim> {
    let mut reclaim_keys = BTreeSet::new();
    let mut reclaim_owners = BTreeMap::new();

    for action in actions {
        if let Action::ReclaimAllocationNetwork { alloc_id } = action {
            if let Some(other) = reclaim_owners.get(alloc_id) {
                return Err(ConflictingAllocationReclaim {
                    alloc_id: alloc_id.clone(),
                    other: *other,
                });
            }
            reclaim_keys.insert(alloc_id.clone());
        } else if let Some((alloc_id, other)) = reclaim_owner(action) {
            if reclaim_keys.contains(alloc_id) {
                return Err(ConflictingAllocationReclaim { alloc_id: alloc_id.clone(), other });
            }
            reclaim_owners.entry(alloc_id.clone()).or_insert(other);
        }
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use overdrive_core::byte_size::ByteSize;
    use overdrive_core::cpu_quantity::CpuQuantity;
    use overdrive_core::reconcilers::Action;

    use super::{ReclaimConflictAction, validate_reconcile_output};
    use overdrive_testing::placement;

    placement!(host_pure);

    /// Empty action vec is trivially valid — no conflicts.
    #[test]
    fn validate_accepts_empty_vec() {
        assert!(validate_reconcile_output(&[]).is_ok());
    }

    fn alloc(name: &str) -> overdrive_core::id::AllocationId {
        overdrive_core::id::AllocationId::new(name).expect("allocation id")
    }

    fn allocation_spec(name: &str) -> overdrive_core::traits::driver::AllocationSpec {
        use overdrive_core::traits::driver::{DriverPayload, Resources, VmPayload};
        overdrive_core::traits::driver::AllocationSpec {
            declared_listeners: Vec::new(),
            alloc: alloc(name),
            identity: overdrive_core::SpiffeId::new(&format!(
                "spiffe://overdrive.local/workload/nd295/alloc/{name}"
            ))
            .expect("SPIFFE ID"),
            driver: DriverPayload::Vm(VmPayload {
                volume: None,
                command: "/bin/true".to_owned(),
                args: Vec::new(),
                kernel: std::path::PathBuf::from("/nd295/kernel"),
                rootfs: std::path::PathBuf::from("/nd295/rootfs.ext4"),
            }),
            resources: Resources {
                cpu: CpuQuantity::from_millis(100),
                memory: ByteSize::new(64 * 1024 * 1024),
            },
            probe_descriptors: Vec::new(),
            transport: None,
        }
    }

    /// The four actions that already own an allocation's cleanup in one
    /// evaluation (FD § "[REF] Lifecycle action — row-neutral reclaim (D-295-R11) — ACCEPTED 2026-09-24" (the validator)), each naming `name`.
    fn owning_actions(name: &str) -> [(ReclaimConflictAction, Action); 4] {
        [
            (
                ReclaimConflictAction::StartAllocation,
                Action::StartAllocation {
                    alloc_id: alloc(name),
                    workload_id: overdrive_core::id::WorkloadId::new("nd295-workload")
                        .expect("workload id"),
                    node_id: overdrive_core::id::NodeId::new("nd295-node").expect("node id"),
                    spec: allocation_spec(name),
                    kind: overdrive_core::aggregate::WorkloadKind::Service,
                },
            ),
            (
                ReclaimConflictAction::RestartAllocation,
                Action::RestartAllocation {
                    alloc_id: alloc(name),
                    spec: allocation_spec(&format!("{name}-successor")),
                    kind: overdrive_core::aggregate::WorkloadKind::Service,
                },
            ),
            (
                ReclaimConflictAction::StopAllocation,
                Action::StopAllocation { alloc_id: alloc(name), terminal: None },
            ),
            (
                ReclaimConflictAction::FinalizeFailed,
                Action::FinalizeFailed { alloc_id: alloc(name), terminal: None },
            ),
        ]
    }

    /// Outcome anchor: OUT-ND295-SHARED-SWITCH
    /// S-ND295-55 — Every leased, unowned, finished allocation is reclaimed from every reconcile path
    /// CONTRACT_SHAPE: pure-function.
    ///
    /// A reclaim may never name an allocation another action names in the
    /// same evaluation: `ReclaimAllocationNetwork` beside `StartAllocation`,
    /// `RestartAllocation` (its predecessor), `StopAllocation`, or
    /// `FinalizeFailed` for the same `alloc_id` is rejected in either order;
    /// beside the same actions for another allocation, alone, or beside a
    /// reclaim of another allocation, it is accepted (FD § "[REF] Lifecycle action — row-neutral reclaim (D-295-R11) — ACCEPTED 2026-09-24" (the validator)).
    #[test]
    fn a_reclaim_beside_another_action_for_the_same_allocation_is_rejected() {
        let reclaim = |name: &str| Action::ReclaimAllocationNetwork { alloc_id: alloc(name) };
        assert!(
            validate_reconcile_output(&[reclaim("nd295-a")]).is_ok(),
            "a reclaim alone is valid"
        );
        assert!(
            validate_reconcile_output(&[reclaim("nd295-a"), reclaim("nd295-b")]).is_ok(),
            "reclaims of distinct allocations are valid together"
        );
        for (kind, owning) in owning_actions("nd295-a") {
            for actions in
                [vec![reclaim("nd295-a"), owning.clone()], vec![owning.clone(), reclaim("nd295-a")]]
            {
                let violation = validate_reconcile_output(&actions).expect_err(&format!(
                    "a reclaim beside {kind:?} for the same allocation must be rejected"
                ));
                assert_eq!(
                    violation.alloc_id,
                    alloc("nd295-a"),
                    "the conflict names the shared allocation, not the {kind:?} successor"
                );
                assert_eq!(
                    violation.other, kind,
                    "the conflict names {kind:?} as the action beside the reclaim, in \
                     either emission order"
                );
            }
            for actions in
                [vec![reclaim("nd295-b"), owning.clone()], vec![owning.clone(), reclaim("nd295-b")]]
            {
                assert!(
                    validate_reconcile_output(&actions).is_ok(),
                    "a reclaim beside {kind:?} for another allocation is valid"
                );
            }
        }
    }
}
