//! `PersistentServiceVipAllocator` — `ServiceVipAllocator` wrapped with
//! redb-backed write-through and bulk-load reconstruction.
//!
//! Per ADR-0049 § Amendments (2026-05-14) this wrapper is **concrete to
//! `ServiceVip`** — no generic across token types. `BackendIdAllocator`
//! never persists, so the symmetry that motivated an earlier
//! `IntentBackedAllocator<T>` generic was rejected.
//!
//! # Write-through ordering — fsync-then-memory
//!
//! Per `.claude/rules/development.md` § "Reconciler I/O" → "Runtime
//! mechanics" the load-bearing invariant is **fsync-then-memory**:
//!
//! 1. Wrap the new entry via `ServiceVipAllocatorEntry::archive_for_store`.
//! 2. Write to the byte-level `IntentStore` (which fsyncs on commit).
//! 3. **Only after** the store write returns Ok, update the in-memory
//!    [`ServiceVipAllocator`] memo + counter.
//!
//! On a crash between steps 2 and 3, the next boot's [`Self::bulk_load`]
//! sees the persisted entry and convergence resumes; no allocation is
//! silently lost. The inverse ordering (memory first, fsync second)
//! would let an acknowledged allocation disappear on crash.
//!
//! # Storage layout
//!
//! Entries live in the existing byte-level `IntentStore`
//! (`crates/overdrive-store-local`) under keys formed by prefixing the
//! 32-byte spec digest with the namespace prefix
//! [`ALLOCATOR_ENTRIES_PREFIX`]. The store's underlying redb `entries`
//! table is unchanged — this is the same pattern as the `Job` aggregate
//! at ADR-0048 § 4b (typed codec on the value; byte-level store
//! surface). Prefix-scan during [`Self::bulk_load`] uses the store's
//! [`IntentStore::scan_prefix`] surface.
//!
//! [`ServiceVipAllocator`]: super::service_vip::ServiceVipAllocator

use std::sync::Arc;

use overdrive_core::codec::EnvelopeError;
use overdrive_core::traits::intent_store::{IntentStore, IntentStoreError};
use thiserror::Error;

use super::entry::{ServiceVipAllocatorEntry, ServiceVipAllocatorEntryV2};
use super::error::ServiceVipAllocatorError;
use super::service_vip::{ServiceSpecDigest, ServiceVip, ServiceVipAllocator};
use super::vip_range::VipRange;

/// redb-key prefix for [`PersistentServiceVipAllocator`] entries.
///
/// Persisted keys take the form `ALLOCATOR_ENTRIES_PREFIX || digest`
/// (39 bytes total: 7-byte prefix `+` 32-byte SHA-256 digest). The
/// prefix scopes the namespace so concurrent intent payloads (jobs,
/// stop sentinels, snapshots) cannot collide with allocator rows in
/// the byte-level `entries` table.
const ALLOCATOR_ENTRIES_PREFIX: &[u8] = b"alloc/v\x01"; // 8 bytes: "alloc/v" + 0x01

/// Compose the storage key for a given service-spec digest.
fn entry_key(digest: &ServiceSpecDigest) -> Vec<u8> {
    let mut key = Vec::with_capacity(ALLOCATOR_ENTRIES_PREFIX.len() + digest.len());
    key.extend_from_slice(ALLOCATOR_ENTRIES_PREFIX);
    key.extend_from_slice(digest);
    key
}

/// Errors from [`PersistentServiceVipAllocator`] operations.
///
/// Pass-through variants per `.claude/rules/development.md` § Errors —
/// the underlying typed error is preserved via `#[from]` so callers can
/// branch on the structured cause without re-parsing `Display` output.
#[derive(Debug, Error)]
pub enum PersistentAllocatorError {
    /// The underlying [`ServiceVipAllocator`] rejected the allocation
    /// (typically pool exhaustion).
    #[error(transparent)]
    Allocator(#[from] ServiceVipAllocatorError),

    /// The byte-level `IntentStore` failed.
    #[error(transparent)]
    Storage(#[from] IntentStoreError),

    /// rkyv envelope serialisation or decode failure. Surfaces on the
    /// write path (archive serialisation) or on [`Self::bulk_load`]
    /// (decoding a persisted entry).
    #[error(transparent)]
    Envelope(#[from] EnvelopeError),

    /// Earned-Trust boot probe (per ADR-0049 § Amendments) detected a
    /// persisted VIP that does not project back within the active
    /// [`VipRange`]. Typically caused by an operator narrowing the
    /// configured `[dataplane.vip_allocator]` range AFTER allocations
    /// were persisted under a wider range — the surviving persisted
    /// entries would now allocate addresses outside the operator's
    /// stated pool.
    ///
    /// Surfaces only from [`Self::bulk_load`]; never from `allocate`
    /// (a fresh allocation cannot produce a VIP outside the range
    /// supplied to the allocator). The control-plane boot path emits
    /// `health.startup.refused` and exits non-zero on this variant;
    /// operator remediation is either (a) restore the wider range or
    /// (b) wipe the on-disk allocator state for a clean re-allocation.
    #[error(
        "Earned Trust probe failed: persisted VIP {vip} is outside the active VipRange — \
         operator likely narrowed [dataplane.vip_allocator].ranges after allocations were persisted"
    )]
    PersistedStateInconsistent {
        /// The first persisted VIP encountered that fails the
        /// projection check. Named verbatim so the operator-facing
        /// `health.startup.refused` event identifies the offending
        /// row without further diagnostics.
        vip: ServiceVip,
    },
}

/// Result alias for [`PersistentAllocatorError`].
pub type Result<T, E = PersistentAllocatorError> = std::result::Result<T, E>;

/// [`ServiceVipAllocator`] wrapped with write-through to a byte-level
/// `IntentStore`.
///
/// See module-level documentation for the fsync-then-memory ordering
/// contract and the storage layout.
pub struct PersistentServiceVipAllocator {
    inner: ServiceVipAllocator,
    store: Arc<dyn IntentStore>,
}

impl PersistentServiceVipAllocator {
    /// Construct a fresh allocator with empty memo over `range`,
    /// backed by `store`. First-boot path.
    ///
    /// Use [`Self::bulk_load`] when restarting against a store that
    /// already carries persisted entries — `new` does NOT consult the
    /// store, so subsequent `allocate` calls would issue VIPs starting
    /// from counter index zero, colliding with prior allocations.
    #[must_use]
    pub fn new(range: VipRange, store: Arc<dyn IntentStore>) -> Self {
        Self { inner: ServiceVipAllocator::new(range), store }
    }

    /// Reconstruct the allocator from persisted entries.
    ///
    /// Iterates every row in the store under
    /// [`ALLOCATOR_ENTRIES_PREFIX`], decodes each through the typed
    /// codec, and replays it into a fresh in-memory
    /// [`ServiceVipAllocator`]. The scan-based allocator (ADR-0049 §
    /// Amendments → 2026-05-19) requires no counter — the next
    /// allocation rescans the configured [`VipRange`] against the
    /// replayed held set to find the first allocatable address.
    ///
    /// # Earned Trust boot probe
    ///
    /// Per ADR-0049 § Amendments, every persisted VIP is checked for
    /// projection back within the active [`VipRange`] BEFORE being
    /// admitted into the in-memory memo. On the first persisted VIP
    /// that fails projection (typically because the operator narrowed
    /// `[dataplane.vip_allocator].ranges` after allocations were
    /// persisted under a wider range), `bulk_load` returns
    /// [`PersistentAllocatorError::PersistedStateInconsistent`]
    /// naming the offending VIP — the control-plane boot path emits
    /// `health.startup.refused` and refuses to start. The probe is
    /// boot-only; the hot `allocate` path never re-checks (a fresh
    /// allocation cannot produce a VIP outside the range supplied to
    /// the allocator). Empty stores pass vacuously.
    ///
    /// # Errors
    ///
    /// * [`PersistentAllocatorError::Storage`] — store read failed.
    /// * [`PersistentAllocatorError::Envelope`] — a persisted row
    ///   failed to decode through the current envelope shape (intent
    ///   layer policy: fail-fast per ADR-0048 § 3).
    /// * [`PersistentAllocatorError::PersistedStateInconsistent`] —
    ///   a persisted VIP does not project back within `range`.
    pub async fn bulk_load(range: VipRange, store: Arc<dyn IntentStore>) -> Result<Self> {
        let rows = store.scan_prefix(ALLOCATOR_ENTRIES_PREFIX).await?;
        let mut inner = ServiceVipAllocator::new(range);
        for (_key, value) in rows {
            let entry = ServiceVipAllocatorEntry::from_store_bytes(&value)?;
            // Earned Trust probe — refuse to admit a persisted VIP
            // that the active range no longer covers. IPv6 entries
            // are also refused at this boundary (Phase 1 allocator
            // is IPv4-only per ADR-0049 § 5); the projection failure
            // surfaces as the same typed error so the operator-
            // facing event is uniform.
            let in_range = entry.vip.try_as_ipv4().is_some_and(|v4| inner.range_contains(v4));
            if !in_range {
                return Err(PersistentAllocatorError::PersistedStateInconsistent {
                    vip: entry.vip,
                });
            }
            inner.restore_entry(entry.spec_digest, entry.vip);
        }
        Ok(Self { inner, store })
    }

    /// Allocate a [`ServiceVip`] for `digest` and persist the
    /// resulting `(digest, vip)` entry to the store.
    ///
    /// # Ordering
    ///
    /// fsync-then-memory:
    /// 1. Compute the candidate `vip` from the in-memory allocator via
    ///    [`ServiceVipAllocator::peek_next_allocation`]. On memo-hit,
    ///    return immediately without a store write (the entry was
    ///    already persisted on the original allocation).
    /// 2. Archive the entry and `put` it through the byte-level store
    ///    (redb fsync on commit).
    /// 3. After the store write returns `Ok`, commit the in-memory
    ///    state (memo insert via [`ServiceVipAllocator::restore_entry`]).
    ///
    /// On a store write failure between steps 2 and 3 the in-memory
    /// state stays unchanged — the next `allocate(digest)` call
    /// rescans the configured [`VipRange`] against the same held set
    /// and retries with the same candidate VIP, which is idempotent
    /// at the store layer (same key, same archived bytes).
    ///
    /// # Errors
    ///
    /// * [`PersistentAllocatorError::Allocator`] — pool exhausted.
    /// * [`PersistentAllocatorError::Envelope`] — archive
    ///   serialisation failed (unreachable in practice).
    /// * [`PersistentAllocatorError::Storage`] — store write failed.
    pub async fn allocate(&mut self, digest: ServiceSpecDigest) -> Result<ServiceVip> {
        // Memo hit short-circuit — already persisted; no store write
        // required.
        if let Some(existing) = self.inner.get(&digest) {
            return Ok(existing);
        }

        // Probe the next allocation from the in-memory allocator
        // WITHOUT committing. If this returns Ok, we know what `vip`
        // to persist; we only commit to the in-memory state after the
        // store write succeeds.
        let vip = self.inner.peek_next_allocation(&digest)?;

        let entry = ServiceVipAllocatorEntryV2 { spec_digest: digest, vip };
        let archived = entry.archive_for_store()?;
        let key = entry_key(&digest);

        // Step 2: fsync. The byte-level store's `put` commits before
        // returning.
        self.store.put(&key, archived.as_ref()).await?;

        // Step 3: after fsync OK, commit in-memory state.
        self.inner.restore_entry(digest, vip);

        Ok(vip)
    }

    /// Return the VIP currently bound to `digest`, if any.
    #[must_use]
    pub fn get(&self, digest: &ServiceSpecDigest) -> Option<ServiceVip> {
        self.inner.get(digest)
    }

    /// Release the entry for `digest`. Idempotent — a missing entry is
    /// not an error; `get` returns `None` post-call regardless of
    /// prior state.
    ///
    /// # Errors
    ///
    /// [`PersistentAllocatorError::Storage`] — store delete failed.
    pub async fn release(&mut self, digest: &ServiceSpecDigest) -> Result<()> {
        // Order: delete in store first, then drop from memo. On a
        // crash between the two, the next bulk_load will not see this
        // digest (the store row is gone) so the in-memory state will
        // also be empty — a release is a state-removal, not an
        // allocation, so eventual consistency is the safe direction
        // (the only way to "lose" a release on crash is to forget it
        // happened, which means the digest reappears in the memo and
        // the operator can release again — idempotent by design).
        let key = entry_key(digest);
        self.store.delete(&key).await?;
        self.inner.release(digest);
        Ok(())
    }

    /// Current number of persisted allocations (memo size). Mirrors
    /// [`ServiceVipAllocator::memo_len`].
    #[must_use]
    pub fn memo_len(&self) -> usize {
        self.inner.memo_len()
    }

    /// Configured pool capacity.
    #[must_use]
    pub fn capacity(&self) -> u64 {
        self.inner.capacity()
    }
}
