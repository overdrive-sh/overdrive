//! GH #295 D-295-R20 — the operator's cleanup-pending projection is a pure
//! function of the allocation's guest-network lease and its row state.
//!
//! The table is exhaustive over every lease state (`None`, `Admitted`,
//! `Retiring`) × every `AllocState` (FD 4575-4591).

#![allow(clippy::doc_markdown, reason = "the exact CONTRACT_SHAPE marker is repository-mandated")]

use overdrive_core::traits::GuestAttachmentLease;
use overdrive_core::traits::observation_store::AllocState;

/// Every `AllocState` today. `expected_cleanup_pending` matches the row state
/// with no wildcard, so a new state breaks this file at compile time and must
/// be added here with its decided meaning.
const ALL_ROW_STATES: [AllocState; 6] = [
    AllocState::Pending,
    AllocState::Running,
    AllocState::Draining,
    AllocState::Suspended,
    AllocState::Terminated,
    AllocState::Failed,
];

/// The FD 4586-4591 table, written as the decision the operator reads.
const fn expected_cleanup_pending(lease: Option<GuestAttachmentLease>, row: AllocState) -> bool {
    match lease {
        // No lease: cleanup finished and the lease was released, or no network
        // was ever leased.
        None => false,
        // A retiring lease is pending whatever the row says — a failed stop
        // leaves the row Running (R10).
        Some(GuestAttachmentLease::Retiring) => true,
        Some(GuestAttachmentLease::Admitted) => match row {
            AllocState::Pending
            | AllocState::Running
            | AllocState::Draining
            | AllocState::Suspended => false,
            AllocState::Failed | AllocState::Terminated => true,
        },
    }
}

/// Outcome anchor: OUT-ND295-SHARED-SWITCH.
/// S-ND295-58 — Cleanup is pending exactly while the lease outlives the running
/// allocation.
/// CONTRACT_SHAPE: pure-function.
#[test]
#[ignore = "pending DELIVER step 07-04 (S-ND295-58)"]
fn cleanup_pending_matches_the_lease_and_row_state_table() {
    let leases = [None, Some(GuestAttachmentLease::Admitted), Some(GuestAttachmentLease::Retiring)];
    let mut rows = Vec::with_capacity(leases.len() * ALL_ROW_STATES.len());
    for lease in leases {
        for row in ALL_ROW_STATES {
            let observed = lease.is_some_and(|held| held.cleanup_pending(row));
            rows.push((lease, row, observed, expected_cleanup_pending(lease, row)));
        }
    }

    let mismatches: Vec<_> = rows
        .iter()
        .filter(|(_, _, observed, expected)| observed != expected)
        .map(|(lease, row, observed, expected)| {
            format!("lease {lease:?} × row {row:?}: observed {observed}, expected {expected}")
        })
        .collect();
    assert!(
        mismatches.is_empty(),
        "cleanup-pending must equal the lease × row-state table:\n{}",
        mismatches.join("\n")
    );

    // The table's shape itself: exactly the two terminal Admitted rows and
    // every Retiring row are pending.
    let pending = rows.iter().filter(|(_, _, _, expected)| *expected).count();
    assert_eq!(pending, 2 + ALL_ROW_STATES.len());
}
