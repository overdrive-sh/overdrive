//! GH #295 D-295-R20 — the operator's cleanup-pending predicate is a pure
//! function of an allocation's guest-network lease and its row state.
//!
//! `GuestAttachmentLease::cleanup_pending(self, row_state)` is a method on a
//! held lease, so its contract is the table's leased rows: every lease state
//! the type has (`Admitted`, `Retiring`) × every `AllocState` (FD § "[REF] Operator status — network cleanup pending (D-295-R20) — ACCEPTED 2026-09-24 (operator behaviour user ruling of the same date)" (`cleanup_pending` and its table)).
//!
//! The table's "no lease" row is not a row of this predicate: there is no
//! lease to call it on. It belongs to the server projection
//! (`handlers::alloc_status` → `AllocStatusRowBody::network_cleanup_pending`),
//! whose S-ND295-59 bodies read it through the HTTP API once a lease is
//! released (`overdrive-control-plane/tests/integration/network_cleanup_pending_status.rs`),
//! so it is not asserted here.

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

/// Every lease state. `expected_cleanup_pending` matches the lease with no
/// wildcard, so a new lease state breaks this file at compile time.
const ALL_LEASES: [GuestAttachmentLease; 2] =
    [GuestAttachmentLease::Admitted, GuestAttachmentLease::Retiring];

/// The leased rows of the cleanup-pending table of FD § "[REF] Operator status — network cleanup pending (D-295-R20) — ACCEPTED 2026-09-24 (operator behaviour user ruling of the same date)", written as the decision the operator reads.
const fn expected_cleanup_pending(lease: GuestAttachmentLease, row: AllocState) -> bool {
    match lease {
        // A retiring lease is pending whatever the row says — a failed stop
        // leaves the row Running (R10).
        GuestAttachmentLease::Retiring => true,
        GuestAttachmentLease::Admitted => match row {
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
fn cleanup_pending_matches_the_lease_and_row_state_table() {
    let mut rows = Vec::with_capacity(ALL_LEASES.len() * ALL_ROW_STATES.len());
    for lease in ALL_LEASES {
        for row in ALL_ROW_STATES {
            rows.push((
                lease,
                row,
                lease.cleanup_pending(row),
                expected_cleanup_pending(lease, row),
            ));
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
