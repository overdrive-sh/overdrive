//! Action shim for `Action::ReleaseServiceVip` per ADR-0049
//! (amended 2026-05-15) — service-vip-allocator step 03-02.
//!
//! Dispatch invokes [`PersistentServiceVipAllocator::release`], which
//! removes the entry from the in-memory memo AND from the IntentStore
//! `allocator_entries` table in fsync-then-memory order (per the
//! allocator's documented contract). The released VIP returns to the
//! pool for reallocation on the next `allocate(&fresh_digest)`.
//!
//! # Lock discipline
//!
//! The allocator is `Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>`
//! — `tokio::sync::Mutex` rather than `parking_lot` because
//! `release().await` crosses an `.await` point on the IntentStore
//! delete (per `.claude/rules/development.md` § "Concurrency & async"
//! → "Never hold a lock across `.await`"). The guard is acquired,
//! `release` is awaited inside the guard so allocate/release calls
//! across concurrent dispatch frames serialise at the IntentStore
//! write boundary (parallel to `allocate`'s serialisation in 02-03d),
//! and the guard drops at the end of this fn before returning.
//!
//! # Idempotency
//!
//! `PersistentServiceVipAllocator::release` is idempotent — releasing
//! a digest not in the memo is a no-op (returns Ok(())). The
//! reconciler's `released_for_terminal` gate (step 03-01) prevents
//! re-emission across ticks, but a duplicate `Action::ReleaseServiceVip`
//! for an already-released digest on the dispatch path does NOT
//! panic; it logs a debug event and returns Ok.

use std::sync::Arc;

use overdrive_core::id::{ContentHash, CorrelationKey};
use overdrive_dataplane::allocators::PersistentServiceVipAllocator;

use super::ShimError;

/// Dispatch one `Action::ReleaseServiceVip`. Calls
/// [`PersistentServiceVipAllocator::release`] under the allocator's
/// `tokio::sync::Mutex` guard, then drops the guard before returning.
///
/// See module docs for the lock discipline and idempotency contract.
///
/// # Errors
///
/// Returns [`ShimError::AllocatorRelease`] when the underlying
/// allocator's IntentStore delete fails. The allocator's typed error
/// is preserved end-to-end via the `#[from]` variant per
/// `.claude/rules/development.md` § Errors → "Pass-through embedding".
/// In Phase 1 the byte-level `IntentStore::delete` is a redb operation
/// and failure is structurally rare (disk full, file corruption); the
/// reconciler runtime's per-action error isolation in
/// [`super::dispatch`] absorbs this and continues with the rest of the
/// action batch.
pub async fn dispatch(
    spec_digest: &ContentHash,
    correlation: &CorrelationKey,
    allocator: &Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>,
) -> Result<(), ShimError> {
    tracing::debug!(
        target: "overdrive_control_plane::action_shim",
        event = "release_service_vip.dispatch",
        spec_digest = %spec_digest,
        correlation = %correlation,
        "dispatching ReleaseServiceVip to allocator"
    );

    // Acquire the allocator guard; release() crosses an `.await` (the
    // IntentStore delete) so we MUST use tokio::sync::Mutex here.
    // Explicit `drop(guard)` after the release call (rather than
    // implicit end-of-scope drop) satisfies clippy::significant_drop_
    // tightening and documents the lock window in source.
    let mut guard = allocator.lock().await;
    guard.release(spec_digest.as_bytes()).await.map_err(ShimError::from)?;
    drop(guard);
    Ok(())
}
