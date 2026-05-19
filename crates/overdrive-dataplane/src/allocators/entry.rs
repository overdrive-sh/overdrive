//! `ServiceVipAllocatorEntry` — rkyv-versioned persistence envelope
//! for a single `(spec_digest, vip)` allocator row.
//!
//! Per ADR-0048 every rkyv-persisted type at a durable-storage boundary
//! ships through a per-type versioned envelope. This module owns the
//! envelope for the [`PersistentServiceVipAllocator`]'s redb-backed
//! state.
//!
//! # Shape (alias-to-payload, UI-02)
//!
//! * [`ServiceVipAllocatorEntryV1`] — the historical V1 payload, with
//!   a `counter_idx` field carried over from the monotonic-counter
//!   allocator shape (rejected 2026-05-19 per ADR-0049 § Amendments).
//!   Preserved verbatim — `FIXTURE_V1` in the schema_evolution test
//!   continues to assert these bytes decode through the envelope.
//! * [`ServiceVipAllocatorEntryV2`] — the current payload. Drops
//!   `counter_idx` since the scan-based allocator does not consume it.
//! * [`ServiceVipAllocatorEntry`] — public alias = `V2` today; rebinds
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
//! * [`ServiceVipAllocatorEntryV2::archive_for_store`] — writer path;
//!   wraps via `Envelope::latest(...)` and rkyv-serialises.
//! * [`ServiceVipAllocatorEntryV2::from_store_bytes`] — reader path;
//!   rkyv-deserialises into the envelope and projects via
//!   `into_latest()`. V1 entries persisted by prior binaries are
//!   transparently up-converted via `From<V1> for V2` (counter_idx is
//!   discarded — it was never consumed by the scan-based allocator).
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

/// The current public payload alias — points at [`ServiceVipAllocatorEntryV2`].
///
/// Public callers use this alias for struct-literal construction:
/// `ServiceVipAllocatorEntry { spec_digest, vip }`. On a future `V3`
/// bump both this alias and [`ServiceVipAllocatorEntryLatest`] move to
/// point at the new payload in a single commit per
/// `.claude/rules/development.md` § "Version-bump procedure".
pub type ServiceVipAllocatorEntry = ServiceVipAllocatorEntryV2;

/// Documentation alias for the latest payload variant — identical to
/// [`ServiceVipAllocatorEntry`] today; preserved across version bumps
/// for callers that explicitly want to name "the latest projection."
pub type ServiceVipAllocatorEntryLatest = ServiceVipAllocatorEntryV2;

/// Historical V1 payload of the [`ServiceVipAllocatorEntry`].
///
/// Carried a `counter_idx` field that fed the monotonic-counter
/// allocator (rejected 2026-05-19 per ADR-0049 § Amendments). The field
/// is preserved here so existing persisted bytes still decode through
/// the envelope; the `From<V1> for V2` conversion discards
/// `counter_idx` because the scan-based allocator does not consume it.
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ServiceVipAllocatorEntryV1 {
    /// Service-spec content digest — the key under which this entry
    /// was persisted in redb.
    pub spec_digest: ServiceSpecDigest,
    /// The allocated VIP.
    pub vip: ServiceVip,
    /// Monotonic counter index at allocation time. Unused by the
    /// scan-based V2 allocator; preserved as a historical input.
    pub counter_idx: u64,
}

/// Current V2 payload of the [`ServiceVipAllocatorEntry`].
///
/// Two fields, both inputs (not derived) per
/// `.claude/rules/development.md` § "Persist inputs, not derived state":
///
/// * `spec_digest` — the 32-byte SHA-256 content hash of the service
///   spec. Identifies which workload owns this VIP allocation.
/// * `vip` — the allocated [`ServiceVip`].
///
/// No `counter_idx` field — the scan-based allocator computes the next
/// allocatable address from the held set at allocate time, never from a
/// persisted counter (ADR-0049 § Amendments → 2026-05-19).
///
/// [`PersistentServiceVipAllocator`]: super::PersistentServiceVipAllocator
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ServiceVipAllocatorEntryV2 {
    /// Service-spec content digest — the key under which this entry is
    /// persisted in redb.
    pub spec_digest: ServiceSpecDigest,
    /// The allocated VIP.
    pub vip: ServiceVip,
}

impl From<ServiceVipAllocatorEntryV1> for ServiceVipAllocatorEntryV2 {
    /// Drop the obsolete `counter_idx` field. The scan-based V2
    /// allocator never consumes it; the V1 binary persisted the value
    /// for a monotonic-counter shape rejected by ADR-0049 § Amendments
    /// → 2026-05-19.
    fn from(v1: ServiceVipAllocatorEntryV1) -> Self {
        Self { spec_digest: v1.spec_digest, vip: v1.vip }
    }
}

/// Codec-internal versioned envelope for the
/// [`ServiceVipAllocatorEntry`] payload.
///
/// **Not re-exported from any `lib.rs`** per ADR-0048 § 2 Layer 1 — the
/// canonical writer path is
/// [`ServiceVipAllocatorEntryV2::archive_for_store`], which goes
/// through `Envelope::latest(...)` internally. Direct variant
/// construction (`ServiceVipAllocatorEntryEnvelope::V<N>(...)`) outside
/// of this module's own [`VersionedEnvelope`] / `From` impls is
/// rejected by `xtask::dst_lint`'s envelope-variant scanner
/// (ADR-0048 § 2 Layer 2).
#[derive(Debug, Clone, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum ServiceVipAllocatorEntryEnvelope {
    /// Historical V1 payload — preserved for forward-compatibility
    /// against on-disk bytes written by prior binaries. New writes
    /// land in [`Self::V2`].
    V1(ServiceVipAllocatorEntryV1),
    /// Current V2 payload (ADR-0049 § Amendments → 2026-05-19; drops
    /// `counter_idx`).
    V2(ServiceVipAllocatorEntryV2),
}

impl VersionedEnvelope for ServiceVipAllocatorEntryEnvelope {
    type Latest = ServiceVipAllocatorEntryV2;

    fn latest(payload: Self::Latest) -> Self {
        Self::V2(payload)
    }

    fn into_latest(self) -> Result<Self::Latest, EnvelopeError> {
        match self {
            Self::V1(v1) => Ok(v1.into()),
            Self::V2(v2) => Ok(v2),
        }
    }

    fn known_discriminants() -> &'static [u8] {
        // V1 carries rkyv discriminant 0 (declaration order — first
        // variant); V2 carries discriminant 1. When V3 is appended,
        // this becomes `&[0, 1, 2]`.
        &[0, 1]
    }

    fn type_name() -> &'static str {
        "ServiceVipAllocatorEntryEnvelope"
    }
}

impl ServiceVipAllocatorEntryV2 {
    /// Archive this entry for persistence through the byte-level
    /// `IntentStore` surface.
    ///
    /// # Postconditions
    ///
    /// On `Ok(bytes)`, `bytes` is the canonical rkyv-archived byte
    /// sequence of `ServiceVipAllocatorEntryEnvelope::V2(self.clone())`.
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
    /// `ServiceVipAllocatorEntryV2` value — the inner field types
    /// (`[u8; 32]`, `ServiceVip`) are all rkyv-derive-friendly — but
    /// the variant is preserved as the structured error surface.
    pub fn archive_for_store(&self) -> Result<AlignedVec, EnvelopeError> {
        let envelope = ServiceVipAllocatorEntryEnvelope::latest(self.clone());
        rkyv::to_bytes::<rkyv::rancor::Error>(&envelope)
            .map_err(|source| EnvelopeError::Malformed { source })
    }

    /// Decode persisted bytes back into a [`ServiceVipAllocatorEntry`].
    ///
    /// V1 entries persisted by prior binaries are transparently
    /// up-converted via `From<V1> for V2` (counter_idx is discarded).
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
