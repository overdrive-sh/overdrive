//! `ViewStore` — runtime-owned storage abstraction for reconciler `View`
//! values per ADR-0035 §2.
//!
//! The `Reconciler` trait surface (after the §18 collapse) does not
//! expose any storage handle. Reconciler authors derive
//! `Serialize + Deserialize + Default + Clone` on the per-reconciler
//! `View` struct and write a sync `reconcile` function over
//! `(desired, actual, view, tick) -> (Vec<Action>, NextView)`. The
//! runtime owns the durable round-trip end-to-end via this port.
//!
//! # Wire shape — bytes on the trait, types in [`ViewStoreExt`]
//!
//! Per ADR-0035 §7 the trait MUST be dyn-compatible so
//! `ReconcilerRuntime::new` can take `Arc<dyn ViewStore>` as its
//! constructor-required port-trait parameter
//! (`.claude/rules/development.md` § "Port-trait dependencies").
//! Generic methods break dyn-compatibility, so the trait operates on
//! pre-encoded CBOR bytes (`Vec<u8>` / `&[u8]`); the typed
//! `bulk_load::<V>` / `write_through::<V>` surface the runtime actually
//! invokes lives on the `Sized`-bounded extension trait
//! [`ViewStoreExt`]. The runtime handles the typed ↔ CBOR translation
//! on either side of the dyn boundary; concrete implementations
//! (`SimViewStore`, `RedbViewStore`) only ever see bytes.
//!
//! # Adapters
//!
//! - **`RedbViewStore`** (step 01-04, `overdrive-control-plane::view_store::redb`):
//!   one redb file per node at `<data_dir>/reconcilers/memory.redb`;
//!   one redb table per reconciler kind keyed on
//!   `TargetResource::display()`; value is a CBOR-encoded blob.
//! - **`SimViewStore`** (step 01-03, `overdrive-sim::adapters::view_store`):
//!   in-memory `BTreeMap<(ReconcilerName, TargetResource), Vec<u8>>`,
//!   with injectable fsync-failure for the
//!   `WriteThroughOrdering` invariant in step 01-07.
//!
//! # Wired into the runtime
//!
//! Step 01-06 closed the loop by making `ReconcilerRuntime::new` take
//! `Arc<dyn ViewStore>` as a mandatory constructor parameter and call
//! `probe()` → `bulk_load()` at register, then `write_through()` after
//! each successful reconcile per ADR-0035 §5.

pub mod redb;

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use overdrive_core::reconciler::TargetResource;

/// Result alias for `ViewStore` operations — keeps call sites short
/// without forcing the long error type on every signature.
pub type Result<T, E = ViewStoreError> = std::result::Result<T, E>;

/// Errors from a `ViewStore` operation. Pass-through embedding via
/// `#[from]` per `.claude/rules/development.md` § Errors.
#[derive(Debug, Error)]
pub enum ViewStoreError {
    /// CBOR encode failure — the `View` could not be serialised.
    /// Should not happen for `View` types that derive `Serialize`
    /// against straightforward shapes; surfaces here on exotic
    /// custom impls only.
    #[error("CBOR encode failed: {0}")]
    Encode(String),

    /// CBOR decode failure — a persisted blob could not be decoded
    /// against the requested `V` type. Indicates schema skew between
    /// the in-memory `View` shape and the on-disk bytes; the runtime
    /// surfaces this as a hard boot failure (Earned-Trust gate).
    #[error("CBOR decode failed: {0}")]
    Decode(String),

    /// The underlying durable write completed but the fsync syscall
    /// failed. Per ADR-0035 §6 `WriteThroughOrdering`: the runtime's
    /// in-memory `BTreeMap` MUST NOT be updated when this fires.
    #[error("fsync failed: {message}")]
    FsyncFailed {
        /// Cause string from the underlying engine (or sim injection).
        message: String,
    },

    /// Underlying I/O error from the storage engine.
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors from the Earned-Trust startup probe per ADR-0035 § Earned Trust.
///
/// The probe writes a sentinel row, fsyncs, reads it back byte-equal,
/// and deletes it. Any failure variant short-circuits boot with a
/// `health.startup.refused` event.
#[derive(Debug, Error)]
pub enum ProbeError {
    /// The probe could not write its sentinel row. Typical causes: a
    /// read-only filesystem, missing parent directory, or a sim
    /// adapter with `inject_fsync_failure()` set.
    #[error("probe write failed: {source}")]
    WriteFailed {
        /// Underlying `ViewStoreError` cause.
        #[source]
        source: ViewStoreError,
    },

    /// The probe wrote and fsynced its sentinel row but the durable
    /// commit reported failure (e.g. disk full, checksum mismatch on
    /// readback, atomic-rename failure).
    #[error("probe commit failed: {source}")]
    CommitFailed {
        /// Underlying `ViewStoreError` cause.
        #[source]
        source: ViewStoreError,
    },

    /// The probe wrote a sentinel row, fsynced, and read back a
    /// non-byte-equal value. Indicates engine corruption between
    /// write and read — reject startup rather than risk operating
    /// against a corrupted store.
    #[error("probe round-trip mismatch: wrote {wrote:?}, read {got:?}")]
    RoundTripMismatch {
        /// Bytes written (probe sentinel).
        wrote: Vec<u8>,
        /// Bytes read back from the store.
        got: Vec<u8>,
    },

    /// The probe wrote and read back successfully but could not delete
    /// its sentinel row. Surfacing this as a probe failure prevents a
    /// store that "works for writes but rejects deletes" from being
    /// brought online silently.
    #[error("probe cleanup failed: {source}")]
    CleanupFailed {
        /// Underlying `ViewStoreError` cause.
        #[source]
        source: ViewStoreError,
    },
}

/// Runtime-owned storage abstraction for reconciler `View` values.
///
/// **Dyn-compatible**: methods operate on pre-encoded CBOR bytes, not
/// on generic `<V>`. The runtime owns the typed ↔ CBOR translation via
/// the [`ViewStoreExt`] sized-bounded extension trait. See
/// `tests/compile_pass/view_store_dyn_compatible.rs` for the
/// trybuild-pinned property.
///
/// All four methods are documented per ADR-0035 §2.
#[async_trait]
pub trait ViewStore: Send + Sync {
    /// Read every persisted `(target, blob)` pair under `reconciler`
    /// as raw CBOR bytes. Called once per reconciler at boot, before
    /// the first tick — the result becomes the runtime's in-memory
    /// `BTreeMap<TargetResource, View>` steady-state read SSOT.
    ///
    /// `reconciler` is `&'static str` rather than `&ReconcilerName`
    /// per the `refactor-reconciler-static-name` RCA: the static
    /// lifetime that `redb::TableDefinition` requires is encoded in
    /// the type system, not recovered at runtime via `Box::leak` or
    /// an interner. Callers pass `Reconciler::NAME` directly (or, for
    /// the runtime's `AnyReconciler` enum-dispatch path,
    /// `AnyReconciler::static_name()`).
    ///
    /// `BTreeMap` not `HashMap` — iteration order must be
    /// deterministic for DST replay
    /// (`.claude/rules/development.md` § "Ordered-collection choice").
    ///
    /// Returns an empty map when the reconciler has no persisted rows
    /// (fresh registration). This is the common case at first boot.
    async fn bulk_load_bytes(
        &self,
        reconciler: &'static str,
    ) -> Result<BTreeMap<TargetResource, Vec<u8>>>;

    /// Write a single `(target, blob)` pair under `reconciler` to
    /// durable storage. Durable (fsync) BEFORE return — per ADR-0035
    /// §5 step 7→8 the runtime's in-memory map update follows after
    /// this call returns `Ok(())`.
    ///
    /// `reconciler` is `&'static str` per the
    /// `refactor-reconciler-static-name` RCA — see [`bulk_load_bytes`]
    /// for the lifetime rationale.
    ///
    /// Failure modes:
    /// - `ViewStoreError::FsyncFailed` — sim injection or real fsync
    ///   error. Per `WriteThroughOrdering` (§6), the underlying store
    ///   MUST NOT have persisted the row when this fires.
    /// - `ViewStoreError::Io` — underlying engine I/O failure.
    async fn write_through_bytes(
        &self,
        reconciler: &'static str,
        target: &TargetResource,
        cbor: &[u8],
    ) -> Result<()>;

    /// Delete a `(reconciler, target)` row. Called when the runtime
    /// observes that a target has been retired (allocation removed,
    /// etc.). Phase 1 deferral acceptable — leaked rows are bounded
    /// by reconciler-kind cardinality, not tick count.
    ///
    /// `reconciler` is `&'static str` per the
    /// `refactor-reconciler-static-name` RCA — see [`bulk_load_bytes`]
    /// for the lifetime rationale.
    ///
    /// Idempotent: deleting a non-existent row succeeds.
    async fn delete(&self, reconciler: &'static str, target: &TargetResource) -> Result<()>;

    /// Earned-Trust startup probe per ADR-0035 § Earned Trust.
    ///
    /// Composition root invariant: write a sentinel row → fsync →
    /// read it back byte-equal → delete the sentinel. Called once at
    /// boot before the first `bulk_load`; on any failure the runtime
    /// emits `health.startup.refused` and exits non-zero.
    ///
    /// On success the store contains no probe-row residue.
    async fn probe(&self) -> std::result::Result<(), ProbeError>;
}

/// Typed surface over the dyn-compatible [`ViewStore`] byte interface.
///
/// The runtime calls `ViewStoreExt::bulk_load::<R::View>(name)` and
/// `ViewStoreExt::write_through(name, target, &view)`; the impl
/// CBOR-encodes / decodes via `ciborium` and dispatches to the
/// underlying byte-level methods on `Arc<dyn ViewStore>`.
///
/// Implemented for every `ViewStore` (blanket impl). Reconciler
/// authors NEVER call this directly — it is the runtime's seam.
#[async_trait]
pub trait ViewStoreExt: ViewStore {
    /// Typed counterpart of `bulk_load_bytes`. CBOR-decodes every
    /// persisted blob under `reconciler` into a typed `View`. A decode
    /// failure on any single row short-circuits with
    /// `ViewStoreError::Decode` — the runtime treats this as a hard
    /// boot failure (schema skew is unrecoverable without operator
    /// intervention).
    ///
    /// `reconciler` is `&'static str` — typically passed as
    /// `R::NAME` for a concrete reconciler `R`, or
    /// `AnyReconciler::static_name()` from the runtime's enum-dispatch
    /// path.
    async fn bulk_load<V>(&self, reconciler: &'static str) -> Result<BTreeMap<TargetResource, V>>
    where
        V: DeserializeOwned + Send;

    /// Typed counterpart of `write_through_bytes`. CBOR-encodes
    /// `view` then dispatches to the byte-level method. Encode
    /// failures surface as `ViewStoreError::Encode` (rare for
    /// straightforward `Serialize` derives).
    ///
    /// `reconciler` is `&'static str` — see [`bulk_load`] for the
    /// lifetime rationale.
    async fn write_through<V>(
        &self,
        reconciler: &'static str,
        target: &TargetResource,
        view: &V,
    ) -> Result<()>
    where
        V: Serialize + Sync;
}

#[async_trait]
impl<T: ViewStore + ?Sized> ViewStoreExt for T {
    async fn bulk_load<V>(&self, reconciler: &'static str) -> Result<BTreeMap<TargetResource, V>>
    where
        V: DeserializeOwned + Send,
    {
        let raw = self.bulk_load_bytes(reconciler).await?;
        let mut out = BTreeMap::new();
        for (target, bytes) in raw {
            let value: V = ciborium::from_reader(bytes.as_slice())
                .map_err(|e| ViewStoreError::Decode(e.to_string()))?;
            out.insert(target, value);
        }
        Ok(out)
    }

    async fn write_through<V>(
        &self,
        reconciler: &'static str,
        target: &TargetResource,
        view: &V,
    ) -> Result<()>
    where
        V: Serialize + Sync,
    {
        let mut buf: Vec<u8> = Vec::new();
        ciborium::into_writer(view, &mut buf).map_err(|e| ViewStoreError::Encode(e.to_string()))?;
        self.write_through_bytes(reconciler, target, &buf).await
    }
}
