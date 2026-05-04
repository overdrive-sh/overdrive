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

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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

    /// Counter incremented on every successful (non-injected,
    /// non-error) `write_through_bytes` call. Exposed via
    /// [`SimViewStore::write_through_count`] so tests can assert that
    /// the runtime's Eq-diff gate actually elides the fsync on
    /// no-op ticks (a reconciler returning `next_view == view`).
    /// Probe-internal `write_through_bytes` calls also increment this;
    /// tests that care reset before the assertion phase via
    /// [`SimViewStore::reset_write_through_count`].
    write_through_count: Arc<AtomicU64>,
}

impl SimViewStore {
    /// Construct an empty `SimViewStore` with no failure injection
    /// configured.
    #[must_use]
    pub fn new() -> Self {
        Self {
            storage: Mutex::new(BTreeMap::new()),
            inject_fsync_failure_flag: Arc::new(AtomicBool::new(false)),
            write_through_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Read the cumulative count of successful `write_through_bytes`
    /// calls against this store. Tests that assert "the runtime did
    /// not call `write_through`" read this; tests that assert "the
    /// runtime called `write_through` exactly once" read it before
    /// and after a tick.
    ///
    /// Probe-internal `write_through` calls (the Earned-Trust
    /// handshake) also bump this counter. Tests that need to ignore
    /// the probe call [`SimViewStore::reset_write_through_count`]
    /// after register completes.
    #[must_use]
    pub fn write_through_count(&self) -> u64 {
        self.write_through_count.load(Ordering::SeqCst)
    }

    /// Reset the cumulative `write_through` counter to zero. Pairs
    /// with [`SimViewStore::write_through_count`] for tests that want
    /// to ignore probe-internal writes during register.
    pub fn reset_write_through_count(&self) {
        self.write_through_count.store(0, Ordering::SeqCst);
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

/// Convert a `&'static str` reconciler name (the byte-level trait's
/// boundary type) into the typed `ReconcilerName` used as the
/// internal storage map key. The contract is: callers pass
/// `Reconciler::NAME` or `AnyReconciler::static_name()`, both of
/// which are literals validated against `ReconcilerName::new`'s
/// `^[a-z][a-z0-9-]{0,62}$` grammar by the trait's doc contract.
/// A future caller passing arbitrary runtime bytes is structurally
/// blocked by the `&'static str` type — the compile-fail fixture
/// `view_store_rejects_owned_string.rs` is the load-bearing gate.
#[allow(clippy::expect_used)]
fn reconciler_name_from_static(name: &'static str) -> ReconcilerName {
    ReconcilerName::new(name).expect(
        "Reconciler::NAME / AnyReconciler::static_name() are validated by the trait contract; \
         callers cannot pass arbitrary runtime bytes via the `&'static str` boundary",
    )
}

#[async_trait]
impl ViewStore for SimViewStore {
    async fn bulk_load_bytes(
        &self,
        reconciler: &'static str,
    ) -> VsResult<BTreeMap<TargetResource, Vec<u8>>> {
        // Convert the `&'static str` boundary parameter to the
        // typed `ReconcilerName` we use as the internal map key.
        // The contract is: callers pass `Reconciler::NAME` (or
        // `AnyReconciler::static_name()`), which by construction is
        // a literal validated against `ReconcilerName::new` — see
        // the `Reconciler::NAME` doc on `overdrive_core::reconciler`.
        // The `expect` therefore can only fire if a caller violates
        // the contract by passing arbitrary runtime bytes; the
        // compile-fail fixture
        // `tests/compile_fail/view_store_rejects_owned_string.rs`
        // closes that hole at the type level.
        let typed = reconciler_name_from_static(reconciler);
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
                (name == &typed).then(|| (target.clone(), bytes.clone()))
            })
            .collect();
        Ok(out)
    }

    async fn write_through_bytes(
        &self,
        reconciler: &'static str,
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
        let typed = reconciler_name_from_static(reconciler);
        // Lock scope tightened per clippy::significant_drop_tightening
        // — single-statement insert, lock drops at end of expression.
        self.storage.lock().insert((typed, target.clone()), cbor.to_vec());
        // Bump the counter only after the durable insert, AFTER the
        // fsync-failure short-circuit above. This makes
        // `write_through_count` a faithful witness to "how many times
        // did the runtime cause a successful fsync against this
        // store" — exactly what the Eq-diff regression test asserts.
        self.write_through_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn delete(&self, reconciler: &'static str, target: &TargetResource) -> VsResult<()> {
        let typed = reconciler_name_from_static(reconciler);
        // Idempotent — `BTreeMap::remove` returning `None` is fine.
        // Lock scope tightened per clippy::significant_drop_tightening.
        let _ = self.storage.lock().remove(&(typed, target.clone()));
        Ok(())
    }

    async fn probe(&self) -> std::result::Result<(), ProbeError> {
        // The probe reconciler name is a compile-time `&'static str`
        // constant — passes through the byte-level surface directly.
        // The probe target still needs constructor validation since
        // it is keyed on the typed `TargetResource`.
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
        self.write_through_bytes(PROBE_RECONCILER, &probe_target, PROBE_PAYLOAD)
            .await
            .map_err(|source| ProbeError::WriteFailed { source })?;

        let loaded = self
            .bulk_load_bytes(PROBE_RECONCILER)
            .await
            .map_err(|source| ProbeError::ReadFailed { source })?;

        let got = loaded.get(&probe_target).cloned().unwrap_or_default();
        if got.as_slice() != PROBE_PAYLOAD {
            return Err(ProbeError::RoundTripMismatch { wrote: PROBE_PAYLOAD.to_vec(), got });
        }

        self.delete(PROBE_RECONCILER, &probe_target)
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

    /// Reconciler name as `&'static str` literal — the byte-level
    /// `ViewStore` surface requires `&'static` per the
    /// `refactor-reconciler-static-name` RCA.
    const N: &str = "job-lifecycle";
    const N_OTHER: &str = "node-drainer";

    fn target(s: &str) -> TargetResource {
        TargetResource::new(s).expect("valid target resource")
    }

    #[tokio::test]
    async fn bulk_load_returns_empty_on_fresh_store() {
        let store = SimViewStore::new();
        let loaded: BTreeMap<TargetResource, Counter> = store.bulk_load(N).await.expect("ok");
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn write_through_then_bulk_load_returns_value() {
        let store = SimViewStore::new();
        let t = target("job/payments");
        let v = Counter { n: 42, label: "x".into() };

        store.write_through(N, &t, &v).await.expect("write ok");

        let loaded: BTreeMap<TargetResource, Counter> = store.bulk_load(N).await.expect("read ok");
        assert_eq!(loaded.get(&t), Some(&v));
    }

    #[tokio::test]
    async fn bulk_load_filters_by_reconciler_name() {
        let store = SimViewStore::new();
        let t = target("job/payments");

        store.write_through(N, &t, &Counter { n: 1, label: "a".into() }).await.expect("write a");
        store
            .write_through(N_OTHER, &t, &Counter { n: 2, label: "b".into() })
            .await
            .expect("write b");

        let loaded_a: BTreeMap<TargetResource, Counter> = store.bulk_load(N).await.expect("read a");
        let loaded_b: BTreeMap<TargetResource, Counter> =
            store.bulk_load(N_OTHER).await.expect("read b");

        assert_eq!(loaded_a.get(&t).map(|c| c.n), Some(1));
        assert_eq!(loaded_b.get(&t).map(|c| c.n), Some(2));
    }

    #[tokio::test]
    async fn delete_removes_from_subsequent_bulk_load() {
        let store = SimViewStore::new();
        let t = target("job/payments");
        let v = Counter::default();

        store.write_through(N, &t, &v).await.expect("write ok");
        store.delete(N, &t).await.expect("delete ok");

        let loaded: BTreeMap<TargetResource, Counter> = store.bulk_load(N).await.expect("read ok");
        assert!(!loaded.contains_key(&t));
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let store = SimViewStore::new();
        let t = target("job/payments");

        // Delete on a never-written key succeeds.
        store.delete(N, &t).await.expect("idempotent delete ok");
    }

    #[tokio::test]
    async fn probe_succeeds_and_leaves_no_residual_rows() {
        let store = SimViewStore::new();
        store.probe().await.expect("probe ok on fresh store");

        // No rows visible under the probe sentinel name. `PROBE_RECONCILER`
        // is itself a `&'static str` constant — flows through the byte
        // surface directly.
        let leftover = store.bulk_load_bytes(PROBE_RECONCILER).await.expect("bulk_load_bytes ok");
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
        let t = target("job/payments");
        let v = Counter { n: 99, label: "should not persist".into() };

        store.inject_fsync_failure();

        let result = store.write_through(N, &t, &v).await;
        assert!(
            matches!(result, Err(ViewStoreError::FsyncFailed { .. })),
            "expected FsyncFailed, got {result:?}"
        );

        // Critical: `WriteThroughOrdering` — storage map must NOT have
        // been mutated by the failed write.
        store.clear_fsync_failure();
        let loaded: BTreeMap<TargetResource, Counter> = store.bulk_load(N).await.expect("read ok");
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
        let t = target("job/payments");
        let v = Counter { n: 1, label: "ok".into() };

        store.inject_fsync_failure();
        let _ = store.write_through(N, &t, &v).await;
        store.clear_fsync_failure();

        store.write_through(N, &t, &v).await.expect("write ok after clear");

        let loaded: BTreeMap<TargetResource, Counter> = store.bulk_load(N).await.expect("read ok");
        assert_eq!(loaded.get(&t), Some(&v));
    }

    /// Counter pinning — fresh store reports zero. The runtime
    /// Eq-diff regression test in
    /// `crates/overdrive-control-plane/tests/integration/
    /// reconciler_runtime_view_store.rs` reads
    /// [`SimViewStore::write_through_count`] to assert "the runtime
    /// did not call `write_through`"; if the accessor were stubbed
    /// out (e.g. `-> 0`), that test would silently pass with no
    /// signal. This unit test pins the live behaviour: write through
    /// → counter increments to exactly N, distinguishing a stubbed
    /// `0` from a stubbed `1` from the real implementation.
    #[tokio::test]
    async fn write_through_count_starts_at_zero_and_increments_per_successful_write() {
        let store = SimViewStore::new();
        assert_eq!(store.write_through_count(), 0, "fresh store must report zero writes");

        let t = target("job/payments");
        let v = Counter { n: 1, label: "first".into() };

        store.write_through(N, &t, &v).await.expect("write 1");
        assert_eq!(store.write_through_count(), 1, "counter must be exactly 1 after one write");

        store.write_through(N, &t, &v).await.expect("write 2");
        assert_eq!(store.write_through_count(), 2, "counter must be exactly 2 after two writes");

        store.write_through(N_OTHER, &t, &v).await.expect("write 3 (other reconciler)");
        assert_eq!(
            store.write_through_count(),
            3,
            "counter aggregates across reconciler names — per-reconciler scoping is the test's job"
        );
    }

    /// `inject_fsync_failure` must NOT bump the counter — the counter
    /// reflects successful writes only. A failed write that incremented
    /// the counter would mislead the Eq-diff regression test.
    #[tokio::test]
    async fn write_through_count_unchanged_when_fsync_failure_injected() {
        let store = SimViewStore::new();
        let t = target("job/payments");
        let v = Counter { n: 1, label: "x".into() };

        store.inject_fsync_failure();
        let result = store.write_through(N, &t, &v).await;
        assert!(result.is_err(), "write must fail under injection");
        assert_eq!(
            store.write_through_count(),
            0,
            "failed write MUST NOT bump the counter — the counter is a witness to successful fsyncs only"
        );
    }

    /// `reset_write_through_count` zeroes the counter; subsequent
    /// writes increment from there. The runtime Eq-diff regression
    /// test calls this after `register` (which probes, bumping the
    /// counter once) so the post-reset assertion is unambiguous.
    #[tokio::test]
    async fn reset_write_through_count_zeroes_and_subsequent_writes_increment_from_zero() {
        let store = SimViewStore::new();
        let t = target("job/payments");
        let v = Counter { n: 1, label: "x".into() };

        store.write_through(N, &t, &v).await.expect("write 1");
        store.write_through(N, &t, &v).await.expect("write 2");
        assert_eq!(store.write_through_count(), 2, "two writes before reset");

        store.reset_write_through_count();
        assert_eq!(store.write_through_count(), 0, "reset must zero the counter");

        store.write_through(N, &t, &v).await.expect("write 3");
        assert_eq!(
            store.write_through_count(),
            1,
            "post-reset writes increment from zero, not from prior count"
        );
    }
}
