// Linux-only: the BPF surface this module mediates does not exist
// on macOS / other targets. `lib.rs` declares `pub mod gc;`
// unconditionally; the file's contents elide cleanly on non-Linux.
#![cfg(target_os = "linux")]
#![allow(clippy::missing_errors_doc)]

//! Orphan-GC sweep over `BACKEND_MAP` (step 4 of ADR-0040 § 2's
//! 5-step HASH_OF_MAPS atomic-swap orchestration).
//!
//! # The orphan problem
//!
//! `EbpfDataplane::update_service` upserts every backend in the new
//! set into BACKEND_MAP under a deterministic `BackendId` derived
//! from `(IPv4, port)`. When a service shrinks — say from
//! `{B1, B2, B3}` to `{B1, B2}` — B3's entry remains in BACKEND_MAP
//! after step 3's outer-pointer update unless something explicitly
//! removes it. That stale entry is an *orphan*: no live service
//! references it, but it still consumes a BACKEND_MAP slot.
//!
//! Without GC the map fills monotonically, eventually hitting
//! `max_entries` and rejecting future inserts. The bug surfaces as
//! a delayed `LoadFailed` from `update_service` once the map
//! saturates — well after the operations that created the orphans.
//!
//! # The sweep
//!
//! Given the union of every active service's `BackendId` set,
//! [`sweep_orphan_backends`] walks BACKEND_MAP's keys, identifies
//! IDs not in the live set, and removes them. The function is
//! pure-data: it takes a mutable handle on BACKEND_MAP and a live
//! set, returns the list of removed IDs (so callers can log /
//! observability-emit), and never reaches outside the map.
//!
//! Idempotent: a re-call with the same live set after a successful
//! sweep removes nothing further. This is the contract S-2.2-10
//! pins.
//!
//! # Failure mode
//!
//! `keys()` and `remove()` are both syscall-backed (`bpf(2)`) and
//! can fail. The function aborts on the first failure and returns
//! the partial removed-list alongside the error — the caller MUST
//! treat partial results as "some orphans removed, others may
//! remain; retry the sweep on the next tick." This is consistent
//! with the rest of the codebase's failure-mode discipline (per
//! `.claude/rules/development.md` § Errors — distinct failure modes
//! get distinct variants).

use std::collections::BTreeSet;

use crate::maps::BackendEntryPod;

/// Errors from the orphan-GC sweep. Distinct from the other
/// dataplane variants because the partial-progress shape on `keys()`
/// or `remove()` failure is the operationally-distinct surface
/// callers branch on.
#[derive(Debug, thiserror::Error)]
pub enum GcError {
    /// A `keys()` iteration step failed. The map is in an unknown
    /// state mid-iteration; the caller should retry the sweep on
    /// the next tick.
    #[error("BACKEND_MAP keys() iteration failed: {source}")]
    KeysIteration {
        #[source]
        source: aya::maps::MapError,
    },
    /// A `remove(&id)` call failed. The map may have lost some but
    /// not all orphans. The error carries the offending ID and the
    /// list of IDs successfully removed before the failure so the
    /// caller can log them.
    #[error("BACKEND_MAP remove(id={offending_id}) failed: {source}")]
    Remove {
        offending_id: u32,
        removed_before_failure: Vec<u32>,
        #[source]
        source: aya::maps::MapError,
    },
}

/// Sweep `BACKEND_MAP` for entries whose `BackendId` is NOT in
/// `live_ids`, removing each. Returns the list of removed IDs in
/// iteration order.
///
/// Iteration order is `aya::maps::HashMap::keys()`'s order, which
/// is "arbitrary" per aya's documentation (the kernel's hash-table
/// traversal order). Tests should assert on the *set* of removed
/// IDs (sorted via `BTreeSet` collection), not the *vec ordering*.
///
/// `live_ids` is a `BTreeSet<u32>` rather than a `HashSet` per
/// `.claude/rules/development.md` § "Ordered-collection choice" —
/// the function is potentially walked by DST harnesses (Slice 04
/// onward), and `BTreeSet`'s deterministic order is the right
/// default even though *this* function only point-accesses via
/// `contains`.
pub fn sweep_orphan_backends<T>(
    backend_map: &mut aya::maps::HashMap<T, u32, BackendEntryPod>,
    live_ids: &BTreeSet<u32>,
) -> Result<Vec<u32>, GcError>
where
    T: std::borrow::BorrowMut<aya::maps::MapData>,
{
    // Phase 1 — collect orphan candidates. We MUST snapshot keys
    // before mutating the map: removing during iteration on a BPF
    // hash map produces undefined kernel behavior (the cursor's
    // next-pointer may dangle).
    let mut orphans: Vec<u32> = Vec::new();
    for key_result in backend_map.keys() {
        let key = key_result.map_err(|source| GcError::KeysIteration { source })?;
        if !live_ids.contains(&key) {
            orphans.push(key);
        }
    }

    // Phase 2 — remove each orphan. Track successes so that on
    // partial failure the caller learns which IDs are already gone.
    let mut removed: Vec<u32> = Vec::with_capacity(orphans.len());
    for orphan in orphans {
        backend_map.remove(&orphan).map_err(|source| GcError::Remove {
            offending_id: orphan,
            removed_before_failure: removed.clone(),
            source,
        })?;
        removed.push(orphan);
    }

    Ok(removed)
}
