//! `SimViewStore` — in-memory `ViewStore` implementation for DST.
//!
//! Sibling to `RedbViewStore` (production, step 01-04). Stores
//! pre-encoded CBOR `Vec<u8>` blobs keyed on
//! `(ReconcilerName, TargetResource)` so the store can hold
//! heterogeneous `View` types across reconciler kinds without an
//! `Any`-shaped registry.
//!
//! Ordering: the storage map is `BTreeMap`, not `HashMap` — `bulk_load`
//! iterates a name-prefixed slice, and DST reproducibility requires a
//! deterministic iteration order
//! (`.claude/rules/development.md` § "Ordered-collection choice").
//!
//! # Failure injection
//!
//! Tests exercising the `WriteThroughOrdering` invariant
//! (ADR-0035 §6 / wave-decisions §D6, step 01-07) need to assert that
//! a failed `write_through` leaves the runtime's in-memory map
//! unchanged. The sim adapter exposes
//! [`SimViewStore::inject_fsync_failure`] /
//! [`SimViewStore::clear_fsync_failure`] handles for this purpose; the
//! production [`RedbViewStore`] (step 01-04) has no such surface.
//!
//! When the failure flag is set:
//! - `write_through_bytes` returns `Err(ViewStoreError::FsyncFailed)`
//!   WITHOUT mutating the underlying CBOR-byte map.
//! - `probe` returns `Err(ProbeError::WriteFailed)` with the same
//!   underlying cause.
//!
//! The flag is one-shot per call shape — `clear_fsync_failure` resets
//! it. This matches the `WriteThroughOrdering` invariant body which
//! injects, asserts non-mutation, then clears and continues.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;

use overdrive_control_plane::view_store::{
    ProbeError, Result as VsResult, ViewStore, ViewStoreError,
};
use overdrive_core::reconciler::{ReconcilerName, TargetResource};

/// Reserved reconciler name the Earned-Trust probe writes its sentinel
/// row under. Validated by `ReconcilerName::new` at construction so any
/// future change to that validator regresses this constant at compile
/// time. The leading-underscore-style isn't permitted by the validator
/// (must start `[a-z]`), so we use a `probe-` prefix that real
/// reconcilers will not collide with by convention.
const PROBE_RECONCILER: &str = "probe-earned-trust";
const PROBE_TARGET: &str = "alloc/probe-sentinel";
const PROBE_PAYLOAD: &[u8] = b"earned-trust-probe-v1";

/// In-memory `ViewStore` for DST.
///
/// Construct via [`SimViewStore::new`]; the constructor returns an
/// empty store (no probe rows, no sentinel state, no failure flags).
/// All concurrent operations are serialised behind a single
/// `parking_lot::Mutex` — the per-test cardinality (single-digit
/// reconcilers, low-tens of targets) makes contention a non-concern.
pub struct SimViewStore {
    /// Storage map keyed on `(ReconcilerName, TargetResource)` with
    /// pre-encoded CBOR bytes as values. `BTreeMap` for deterministic
    /// iteration order under DST replay
    /// (`.claude/rules/development.md` § "Ordered-collection choice").
    storage: Mutex<BTreeMap<(ReconcilerName, TargetResource), Vec<u8>>>,

    /// One-shot fsync-failure injection flag. When set, the next
    /// `write_through_bytes` (or `probe`) call returns
    /// `Err(ViewStoreError::FsyncFailed)` WITHOUT mutating `storage`.
    /// `clear_fsync_failure` resets to default success behaviour.
    /// Wrapped in `Arc<AtomicBool>` so cloned references stay
    /// coherent across tasks (callers commonly hand the store to
    /// background test tasks).
    inject_fsync_failure_flag: Arc<AtomicBool>,
}

impl SimViewStore {
    /// Construct an empty `SimViewStore` with no failure injection
    /// configured.
    #[must_use]
    pub fn new() -> Self {
        Self {
            storage: Mutex::new(BTreeMap::new()),
            inject_fsync_failure_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Configure the next `write_through_bytes` (or `probe`) call to
    /// return `Err(ViewStoreError::FsyncFailed)` WITHOUT persisting
    /// the row. Used by the `WriteThroughOrdering` invariant in step
    /// 01-07 to assert the runtime's in-memory `BTreeMap` is
    /// unchanged when the underlying durable write fails.
    ///
    /// The flag stays set across multiple subsequent calls until
    /// `clear_fsync_failure` resets it.
    pub fn inject_fsync_failure(&self) {
        self.inject_fsync_failure_flag.store(true, Ordering::SeqCst);
    }

    /// Reset to default success behaviour. Pairs with
    /// `inject_fsync_failure`.
    pub fn clear_fsync_failure(&self) {
        self.inject_fsync_failure_flag.store(false, Ordering::SeqCst);
    }

    /// Read the current fsync-failure injection flag. Helper for
    /// internal short-circuit; not part of the test-facing surface.
    fn fsync_failure_active(&self) -> bool {
        self.inject_fsync_failure_flag.load(Ordering::SeqCst)
    }
}

impl Default for SimViewStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ViewStore for SimViewStore {
    async fn bulk_load_bytes(
        &self,
        reconciler: &ReconcilerName,
    ) -> VsResult<BTreeMap<TargetResource, Vec<u8>>> {
        // Filter the storage map by `reconciler` and drop the prefix
        // from the key — the returned map is keyed on
        // `TargetResource` only, matching the trait contract.
        //
        // Lock scope tightened to a single `collect` per
        // clippy::significant_drop_tightening — the iterator yields
        // owned `(TargetResource, Vec<u8>)` pairs, the lock guard
        // drops at the end of the call expression.
        let out: BTreeMap<TargetResource, Vec<u8>> = self
            .storage
            .lock()
            .iter()
            .filter_map(|((name, target), bytes)| {
                (name == reconciler).then(|| (target.clone(), bytes.clone()))
            })
            .collect();
        Ok(out)
    }

    async fn write_through_bytes(
        &self,
        reconciler: &ReconcilerName,
        target: &TargetResource,
        cbor: &[u8],
    ) -> VsResult<()> {
        if self.fsync_failure_active() {
            // CRITICAL: the storage map MUST NOT be mutated when the
            // fsync-failure injection fires. `WriteThroughOrdering`
            // (ADR-0035 §6) asserts on this exact invariant.
            return Err(ViewStoreError::FsyncFailed {
                message: "sim injection: fsync failure".to_string(),
            });
        }
        // Lock scope tightened per clippy::significant_drop_tightening
        // — single-statement insert, lock drops at end of expression.
        self.storage.lock().insert((reconciler.clone(), target.clone()), cbor.to_vec());
        Ok(())
    }

    async fn delete(&self, reconciler: &ReconcilerName, target: &TargetResource) -> VsResult<()> {
        // Idempotent — `BTreeMap::remove` returning `None` is fine.
        // Lock scope tightened per clippy::significant_drop_tightening.
        let _ = self.storage.lock().remove(&(reconciler.clone(), target.clone()));
        Ok(())
    }

    async fn probe(&self) -> std::result::Result<(), ProbeError> {
        // Reserved sentinel coordinates. Construction failures here
        // would indicate a bug in `ReconcilerName::new` /
        // `TargetResource::new` against constants we control — surface
        // as a `WriteFailed` with an `Io` cause for visibility, but
        // this branch should never fire in practice.
        let probe_name =
            ReconcilerName::new(PROBE_RECONCILER).map_err(|e| ProbeError::WriteFailed {
                source: ViewStoreError::Io(std::io::Error::other(format!(
                    "probe reconciler name invalid: {e}"
                ))),
            })?;
        let probe_target =
            TargetResource::new(PROBE_TARGET).map_err(|e| ProbeError::WriteFailed {
                source: ViewStoreError::Io(std::io::Error::other(format!(
                    "probe target invalid: {e}"
                ))),
            })?;

        // Honour the fsync-failure injection — the probe is exactly
        // the kind of operation a sim test wants to assert against.
        if self.fsync_failure_active() {
            return Err(ProbeError::WriteFailed {
                source: ViewStoreError::FsyncFailed {
                    message: "sim injection: fsync failure during probe".to_string(),
                },
            });
        }

        // Write → read-back → delete, all under sequential locks. The
        // probe path is short and serial; no need to interleave with
        // other ops.
        self.write_through_bytes(&probe_name, &probe_target, PROBE_PAYLOAD)
            .await
            .map_err(|source| ProbeError::WriteFailed { source })?;

        let loaded = self
            .bulk_load_bytes(&probe_name)
            .await
            .map_err(|source| ProbeError::CommitFailed { source })?;

        let got = loaded.get(&probe_target).cloned().unwrap_or_default();
        if got.as_slice() != PROBE_PAYLOAD {
            return Err(ProbeError::RoundTripMismatch { wrote: PROBE_PAYLOAD.to_vec(), got });
        }

        self.delete(&probe_name, &probe_target)
            .await
            .map_err(|source| ProbeError::CleanupFailed { source })?;

        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use overdrive_control_plane::view_store::{ViewStoreError, ViewStoreExt};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
    struct Counter {
        n: u64,
        label: String,
    }

    fn name(s: &str) -> ReconcilerName {
        ReconcilerName::new(s).expect("valid reconciler name")
    }

    fn target(s: &str) -> TargetResource {
        TargetResource::new(s).expect("valid target resource")
    }

    #[tokio::test]
    async fn bulk_load_returns_empty_on_fresh_store() {
        let store = SimViewStore::new();
        let loaded: BTreeMap<TargetResource, Counter> =
            store.bulk_load(&name("job-lifecycle")).await.expect("ok");
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn write_through_then_bulk_load_returns_value() {
        let store = SimViewStore::new();
        let n = name("job-lifecycle");
        let t = target("job/payments");
        let v = Counter { n: 42, label: "x".into() };

        store.write_through(&n, &t, &v).await.expect("write ok");

        let loaded: BTreeMap<TargetResource, Counter> = store.bulk_load(&n).await.expect("read ok");
        assert_eq!(loaded.get(&t), Some(&v));
    }

    #[tokio::test]
    async fn bulk_load_filters_by_reconciler_name() {
        let store = SimViewStore::new();
        let a = name("job-lifecycle");
        let b = name("node-drainer");
        let t = target("job/payments");

        store.write_through(&a, &t, &Counter { n: 1, label: "a".into() }).await.expect("write a");
        store.write_through(&b, &t, &Counter { n: 2, label: "b".into() }).await.expect("write b");

        let loaded_a: BTreeMap<TargetResource, Counter> =
            store.bulk_load(&a).await.expect("read a");
        let loaded_b: BTreeMap<TargetResource, Counter> =
            store.bulk_load(&b).await.expect("read b");

        assert_eq!(loaded_a.get(&t).map(|c| c.n), Some(1));
        assert_eq!(loaded_b.get(&t).map(|c| c.n), Some(2));
    }

    #[tokio::test]
    async fn delete_removes_from_subsequent_bulk_load() {
        let store = SimViewStore::new();
        let n = name("job-lifecycle");
        let t = target("job/payments");
        let v = Counter::default();

        store.write_through(&n, &t, &v).await.expect("write ok");
        store.delete(&n, &t).await.expect("delete ok");

        let loaded: BTreeMap<TargetResource, Counter> = store.bulk_load(&n).await.expect("read ok");
        assert!(!loaded.contains_key(&t));
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let store = SimViewStore::new();
        let n = name("job-lifecycle");
        let t = target("job/payments");

        // Delete on a never-written key succeeds.
        store.delete(&n, &t).await.expect("idempotent delete ok");
    }

    #[tokio::test]
    async fn probe_succeeds_and_leaves_no_residual_rows() {
        let store = SimViewStore::new();
        store.probe().await.expect("probe ok on fresh store");

        // No rows visible under the probe sentinel name.
        let probe_name = ReconcilerName::new(PROBE_RECONCILER).expect("valid");
        let leftover = store.bulk_load_bytes(&probe_name).await.expect("bulk_load_bytes ok");
        assert!(leftover.is_empty(), "probe must leave no residual rows under its sentinel name");
    }

    #[tokio::test]
    async fn probe_succeeds_repeatedly() {
        let store = SimViewStore::new();
        store.probe().await.expect("probe 1 ok");
        store.probe().await.expect("probe 2 ok");
        store.probe().await.expect("probe 3 ok");
    }

    #[tokio::test]
    async fn inject_fsync_failure_makes_next_write_through_fail() {
        let store = SimViewStore::new();
        let n = name("job-lifecycle");
        let t = target("job/payments");
        let v = Counter { n: 99, label: "should not persist".into() };

        store.inject_fsync_failure();

        let result = store.write_through(&n, &t, &v).await;
        assert!(
            matches!(result, Err(ViewStoreError::FsyncFailed { .. })),
            "expected FsyncFailed, got {result:?}"
        );

        // Critical: `WriteThroughOrdering` — storage map must NOT have
        // been mutated by the failed write.
        store.clear_fsync_failure();
        let loaded: BTreeMap<TargetResource, Counter> = store.bulk_load(&n).await.expect("read ok");
        assert!(!loaded.contains_key(&t), "fsync-failed write must NOT have persisted the row");
    }

    #[tokio::test]
    async fn inject_fsync_failure_makes_probe_fail() {
        let store = SimViewStore::new();
        store.inject_fsync_failure();

        let result = store.probe().await;
        assert!(
            matches!(result, Err(ProbeError::WriteFailed { .. })),
            "expected ProbeError::WriteFailed, got {result:?}"
        );
    }

    #[tokio::test]
    async fn clear_fsync_failure_restores_default_behaviour() {
        let store = SimViewStore::new();
        let n = name("job-lifecycle");
        let t = target("job/payments");
        let v = Counter { n: 1, label: "ok".into() };

        store.inject_fsync_failure();
        let _ = store.write_through(&n, &t, &v).await;
        store.clear_fsync_failure();

        store.write_through(&n, &t, &v).await.expect("write ok after clear");

        let loaded: BTreeMap<TargetResource, Counter> = store.bulk_load(&n).await.expect("read ok");
        assert_eq!(loaded.get(&t), Some(&v));
    }
}
