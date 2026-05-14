//! `ServiceVipAllocatorEntry` — rkyv-versioned persistence envelope
//! for a single `(spec_digest, vip, counter_idx)` allocator row.
//!
//! Per ADR-0048 every rkyv-persisted type at a durable-storage boundary
//! ships through a per-type versioned envelope. This module owns the
//! envelope for the [`PersistentServiceVipAllocator`]'s redb-backed
//! state.
//!
//! # Shape (alias-to-payload, UI-02)
//!
//! * [`ServiceVipAllocatorEntryV1`] — the V1 payload struct.
//! * [`ServiceVipAllocatorEntry`] — public alias = `V1` today; rebinds
//!   to `V<N+1>` on a future version bump (callers continue to use
//!   `ServiceVipAllocatorEntry { ... }` struct-literal syntax).
//! * [`ServiceVipAllocatorEntryLatest`] — documentation alias for "the
//!   latest payload."
//! * [`ServiceVipAllocatorEntryEnvelope`] — codec-internal envelope
//!   enum. `pub` (rustc E0446 prevents `pub(crate)` under the `pub`
//!   `VersionedEnvelope` trait) but **NOT re-exported** from
//!   `overdrive-dataplane::lib` per ADR-0048 § 2 Layer 1.
//!
//! # Codec
//!
//! The persistence-boundary code goes through the typed codec methods
//! on [`ServiceVipAllocatorEntry`]:
//!
//! * [`ServiceVipAllocatorEntryV1::archive_for_store`] — writer path;
//!   wraps via `Envelope::latest(...)` and rkyv-serialises.
//! * [`ServiceVipAllocatorEntryV1::from_store_bytes`] — reader path;
//!   rkyv-deserialises into the envelope and projects via
//!   `into_latest()`.
//!
//! The redb table is byte-level; the typed codec is the SOLE wrapping
//! site. Mirrors the `Job` aggregate codec pattern at
//! `crates/overdrive-core/src/aggregate/mod.rs` (ADR-0048 § 4b).
//!
//! [`PersistentServiceVipAllocator`]: super::PersistentServiceVipAllocator

use overdrive_core::codec::{EnvelopeError, VersionedEnvelope, decode_envelope_bytes};
use overdrive_core::id::ServiceVip;
use rkyv::util::AlignedVec;

use super::service_vip::ServiceSpecDigest;

/// The current public payload alias — points at [`ServiceVipAllocatorEntryV1`].
///
/// Public callers use this alias for struct-literal construction:
/// `ServiceVipAllocatorEntry { spec_digest, vip, counter_idx }`. On a
/// future `V2` bump both this alias and
/// [`ServiceVipAllocatorEntryLatest`] move to point at the new payload
/// in a single commit per `.claude/rules/development.md` § "Version-bump
/// procedure".
pub type ServiceVipAllocatorEntry = ServiceVipAllocatorEntryV1;

/// Documentation alias for the latest payload variant — identical to
/// [`ServiceVipAllocatorEntry`] today; preserved across version bumps
/// for callers that explicitly want to name "the latest projection."
pub type ServiceVipAllocatorEntryLatest = ServiceVipAllocatorEntryV1;

/// Inner V1 payload of the [`ServiceVipAllocatorEntry`].
///
/// Persisted to redb under the per-digest key for the
/// [`PersistentServiceVipAllocator`] (step 01-03). Three fields, all
/// inputs (not derived):
///
/// * `spec_digest` — the 32-byte SHA-256 content hash of the service
///   spec. Identifies which workload owns this VIP allocation.
/// * `vip` — the allocated [`ServiceVip`].
/// * `counter_idx` — monotonic counter position at the time of
///   allocation. Reconstructed counter on restart is
///   `max(counter_idx) + 1` per § "Persist inputs, not derived state"
///   (`.claude/rules/development.md`): we persist the index the
///   allocator emitted, never the "next index to emit" — that's a
///   derived quantity recomputed at bulk_load time.
///
/// [`PersistentServiceVipAllocator`]: super::PersistentServiceVipAllocator
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ServiceVipAllocatorEntryV1 {
    /// Service-spec content digest — the key under which this entry is
    /// persisted in redb.
    pub spec_digest: ServiceSpecDigest,
    /// The allocated VIP.
    pub vip: ServiceVip,
    /// Monotonic counter index at allocation time. Used to
    /// reconstruct the allocator's `next_idx` on restart via
    /// `max(counter_idx) + 1`.
    pub counter_idx: u64,
}

/// Codec-internal versioned envelope for the
/// [`ServiceVipAllocatorEntry`] payload.
///
/// **Not re-exported from any `lib.rs`** per ADR-0048 § 2 Layer 1 — the
/// canonical writer path is
/// [`ServiceVipAllocatorEntryV1::archive_for_store`], which goes
/// through `Envelope::latest(...)` internally. Direct variant
/// construction (`ServiceVipAllocatorEntryEnvelope::V1(...)`) outside
/// of this module's own [`VersionedEnvelope`] impl is rejected by
/// `xtask::dst_lint`'s envelope-variant scanner (ADR-0048 § 2 Layer 2).
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum ServiceVipAllocatorEntryEnvelope {
    /// V1 payload — the only variant today. Future versions are
    /// appended; existing discriminants stay stable per rkyv's
    /// declaration-order tagging (ADR-0048).
    V1(ServiceVipAllocatorEntryV1),
}

impl VersionedEnvelope for ServiceVipAllocatorEntryEnvelope {
    type Latest = ServiceVipAllocatorEntryV1;

    fn latest(payload: Self::Latest) -> Self {
        Self::V1(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1),
        }
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 carries rkyv discriminant 0 (declaration order — first
        // variant). When V2 is appended, this becomes `&[0, 1]`.
        &[0]
    }

    fn type_name() -> &'static str {
        "ServiceVipAllocatorEntryEnvelope"
    }
}

impl ServiceVipAllocatorEntryV1 {
    /// Archive this entry for persistence through the byte-level
    /// `IntentStore` surface.
    ///
    /// # Postconditions
    ///
    /// On `Ok(bytes)`, `bytes` is the canonical rkyv-archived byte
    /// sequence of `ServiceVipAllocatorEntryEnvelope::V1(self.clone())`.
    /// Two archivals of the same logical entry produce byte-identical
    /// output (rkyv canonicalisation).
    ///
    /// # Observable invariants
    ///
    /// `ServiceVipAllocatorEntry::from_store_bytes(&self.archive_for_store()?)`
    /// returns `Ok(self_owned)` bit-equivalent to `self`. The envelope
    /// wrap is internal — callers never name
    /// [`ServiceVipAllocatorEntryEnvelope`].
    ///
    /// # Errors
    ///
    /// Returns [`EnvelopeError::Malformed`] when the rkyv serialiser
    /// itself fails. Unreachable in practice for any
    /// `ServiceVipAllocatorEntryV1` value — the inner field types
    /// (`[u8; 32]`, `ServiceVip`, `u64`) are all rkyv-derive-friendly
    /// — but the variant is preserved as the structured error surface.
    pub fn archive_for_store(&self) -> Result<AlignedVec, EnvelopeError> {
        let envelope = ServiceVipAllocatorEntryEnvelope::latest(self.clone());
        rkyv::to_bytes::<rkyv::rancor::Error>(&envelope)
            .map_err(|source| EnvelopeError::Malformed { source })
    }

    /// Decode persisted bytes back into a [`ServiceVipAllocatorEntry`].
    ///
    /// # Edge cases
    ///
    /// * Empty / truncated `bytes` → [`EnvelopeError::Malformed`] (rkyv
    ///   validator rejection).
    /// * Bytes from a writer that has bumped the envelope to `V<N+1>`
    ///   while this binary knows only up to `V<N>` →
    ///   [`EnvelopeError::UnknownVersion`] (surfaced by the pre-decode
    ///   probe; structured remediation in operator-facing diagnostics).
    /// * Corrupt bytes → [`EnvelopeError::Malformed`].
    pub fn from_store_bytes(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        decode_envelope_bytes::<ServiceVipAllocatorEntryEnvelope>(bytes)
    }
}
