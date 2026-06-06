//! `SimJournalStore` — in-memory `JournalStore` implementation for DST.
//!
//! Sibling to `RedbJournalStore` (production, step 01-04). Stores
//! pre-encoded CBOR `Vec<u8>` blobs keyed on `(WorkflowId, u32)` so the
//! sim has byte-for-byte storage parity with the production redb adapter
//! (ADR-0063 §3) — append round-trips through the same `ciborium` codec
//! the production path uses, catching any codec skew at the sim layer.
//!
//! Ordering: the storage map is `BTreeMap`, not `HashMap`. `load_journal`
//! is a range scan over `(id, *)` and DST reproducibility requires a
//! deterministic iteration order (`.claude/rules/development.md`
//! § "Ordered-collection choice"). The `(WorkflowId, u32)` tuple key
//! gives the ascending-step ordering for free, mirroring redb's tuple-key
//! range-scan shape.
//!
//! # Failure injection
//!
//! The `WorkflowJournalWriteOrdering` invariant (ADR-0063 §4 / step
//! 01-06) asserts that a failed `append` leaves the journal unobservable
//! — the entry is neither persisted nor returned by a later
//! `load_journal`. The sim exposes
//! [`SimJournalStore::inject_fsync_failure`] /
//! [`SimJournalStore::clear_fsync_failure`] for this; the production
//! [`RedbJournalStore`] (step 01-04) has no such surface.
//!
//! When the failure flag is set:
//! - `append` returns `Err(JournalStoreError::FsyncFailed)` WITHOUT
//!   inserting into the underlying map.
//! - `probe` returns `Err(ProbeError::WriteFailed)` with the same
//!   underlying cause.
//!
//! The flag is sticky until [`SimJournalStore::clear_fsync_failure`]
//! resets it — matching the invariant body which injects, asserts
//! non-observability, then clears and continues.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use parking_lot::Mutex;

use overdrive_control_plane::journal::{
    JournalCommand, JournalStore, JournalStoreError, LoadedEntry, ProbeError, Result as JsResult,
    WorkflowId,
};
use overdrive_core::workflow::WorkflowResult;

/// Reserved workflow id the Earned-Trust probe writes its sentinel entry
/// under. Validated by `WorkflowId::new` at construction so any future
/// tightening of that validator regresses this constant at compile time
/// (caught by the `probe_sentinel_id_is_valid` unit test).
const PROBE_WORKFLOW_ID: &str = "probe-wf-earned-trust";

/// In-memory `JournalStore` for DST.
///
/// Construct via [`SimJournalStore::new`]; the constructor returns an
/// empty store (no entries, no failure flag). All operations serialise
/// behind a single `parking_lot::Mutex` — per-test cardinality
/// (single-digit instances, low-tens of entries) makes contention a
/// non-concern, matching `SimViewStore`.
pub struct SimJournalStore {
    /// Storage map keyed on `(WorkflowId, step)` with pre-encoded CBOR
    /// bytes as values. `BTreeMap` for the deterministic ascending-step
    /// range scan `load_journal` relies on
    /// (`.claude/rules/development.md` § "Ordered-collection choice").
    storage: Mutex<BTreeMap<(WorkflowId, u32), Vec<u8>>>,

    /// Sticky fsync-failure injection flag. When set, the next (and every
    /// subsequent) `append` / `probe` call returns
    /// `Err(JournalStoreError::FsyncFailed)` WITHOUT mutating `storage`,
    /// until `clear_fsync_failure` resets it. Wrapped in
    /// `Arc<AtomicBool>` so cloned references stay coherent across tasks.
    inject_fsync_failure_flag: Arc<AtomicBool>,
}

impl SimJournalStore {
    /// Construct an empty `SimJournalStore` with no failure injection
    /// configured.
    #[must_use]
    pub fn new() -> Self {
        Self {
            storage: Mutex::new(BTreeMap::new()),
            inject_fsync_failure_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Arm fsync-failure injection: the next `append` (and `probe`)
    /// returns `Err(JournalStoreError::FsyncFailed)` WITHOUT persisting,
    /// until [`clear_fsync_failure`](Self::clear_fsync_failure) resets
    /// it. Used by the `WorkflowJournalWriteOrdering` invariant (step
    /// 01-06) to assert a failed append leaves the journal
    /// unobservable.
    pub fn inject_fsync_failure(&self) {
        self.inject_fsync_failure_flag.store(true, Ordering::SeqCst);
    }

    /// Clear a previously-armed fsync failure, restoring normal
    /// success behaviour for subsequent `append` / `probe` calls.
    pub fn clear_fsync_failure(&self) {
        self.inject_fsync_failure_flag.store(false, Ordering::SeqCst);
    }

    /// Encode an entry to CBOR bytes via `ciborium` — the same codec the
    /// production redb adapter uses, so the sim catches codec skew.
    fn encode(entry: &LoadedEntry) -> JsResult<Vec<u8>> {
        let mut buf: Vec<u8> = Vec::new();
        ciborium::into_writer(entry, &mut buf)
            .map_err(|e| JournalStoreError::Encode(e.to_string()))?;
        Ok(buf)
    }

    /// Decode CBOR bytes back into a `LoadedEntry`.
    fn decode(bytes: &[u8]) -> JsResult<LoadedEntry> {
        ciborium::from_reader(bytes).map_err(|e| JournalStoreError::Decode(e.to_string()))
    }

    /// The next step index for `workflow_id`. Append order maps 1:1 to
    /// ascending step per the [`JournalStore::append`] contract, and an
    /// instance's steps are contiguous from 0 (`append` is the sole writer;
    /// no real instance deletes entries), so the next step is
    /// `last_step + 1`. Derived by a reverse peek at the back of the
    /// `(id, 0)..=(id, u32::MAX)` `BTreeMap` range — `.next_back()` is an
    /// O(log N) reverse cursor seek, mirroring the production redb adapter's
    /// `Range::next_back` derivation for DST contract parity. An empty range
    /// yields step 0.
    fn next_step(storage: &BTreeMap<(WorkflowId, u32), Vec<u8>>, workflow_id: &WorkflowId) -> u32 {
        let lo = (workflow_id.clone(), 0u32);
        let hi = (workflow_id.clone(), u32::MAX);
        match storage.range(lo..=hi).next_back() {
            None => 0,
            Some(((_id, last_step), _value)) => last_step.checked_add(1).unwrap_or_else(|| {
                unreachable!("a single instance cannot exceed u32::MAX entries")
            }),
        }
    }
}

impl Default for SimJournalStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl JournalStore for SimJournalStore {
    async fn append(&self, workflow_id: &WorkflowId, entry: &LoadedEntry) -> JsResult<()> {
        // Encode BEFORE taking the lock / checking injection so an encode
        // failure surfaces cleanly without mutating any state.
        let bytes = Self::encode(entry)?;

        // Injection: fail the append WITHOUT persisting. Per ADR-0063 §4
        // the entry must not become observable when fsync fails.
        if self.inject_fsync_failure_flag.load(Ordering::SeqCst) {
            return Err(JournalStoreError::FsyncFailed {
                message: "injected fsync failure (SimJournalStore)".to_string(),
            });
        }

        let mut storage = self.storage.lock();
        let step = Self::next_step(&storage, workflow_id);
        storage.insert((workflow_id.clone(), step), bytes);
        drop(storage);
        Ok(())
    }

    async fn load_journal(&self, workflow_id: &WorkflowId) -> JsResult<Vec<LoadedEntry>> {
        let lo = (workflow_id.clone(), 0u32);
        let hi = (workflow_id.clone(), u32::MAX);
        let storage = self.storage.lock();
        // BTreeMap range scan yields keys in ascending `(id, step)` order;
        // filtered to this instance, that IS ascending step order. Decode
        // into an owned Vec, then drop the guard before returning so the
        // lock is held only for the scan (significant_drop_tightening).
        let mut out = Vec::with_capacity(storage.range(lo.clone()..=hi.clone()).count());
        for (_key, bytes) in storage.range(lo..=hi) {
            out.push(Self::decode(bytes)?);
        }
        drop(storage);
        Ok(out)
    }

    async fn probe(&self) -> std::result::Result<(), ProbeError> {
        // Honour the injection flag — a sim probe under injected failure
        // refuses startup exactly as the real adapter would on a
        // read-only / broken substrate.
        if self.inject_fsync_failure_flag.load(Ordering::SeqCst) {
            return Err(ProbeError::WriteFailed {
                source: JournalStoreError::FsyncFailed {
                    message: "injected fsync failure (SimJournalStore probe)".to_string(),
                },
            });
        }

        let probe_id = WorkflowId::new(PROBE_WORKFLOW_ID)
            .unwrap_or_else(|_| unreachable!("PROBE_WORKFLOW_ID is a valid instance id"));
        let sentinel =
            LoadedEntry::Command(JournalCommand::Terminal { result: WorkflowResult::Success });
        let wrote = Self::encode(&sentinel).map_err(|source| ProbeError::WriteFailed { source })?;

        // Write → read back byte-equal → delete, all under one lock so
        // no concurrent op can interleave with the sentinel. The guard is
        // dropped explicitly before each return (significant_drop_tightening).
        let key = (probe_id, 0u32);
        let mut storage = self.storage.lock();
        storage.insert(key.clone(), wrote.clone());
        let got = storage
            .get(&key)
            .cloned()
            .unwrap_or_else(|| unreachable!("sentinel was just inserted under the same lock"));
        storage.remove(&key);
        drop(storage);
        if got != wrote {
            return Err(ProbeError::RoundTripMismatch { wrote, got });
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use overdrive_control_plane::journal::JournalNotification;
    use overdrive_core::id::ContentHash;
    use overdrive_core::workflow::{SignalKey, SignalValue};
    use proptest::prelude::*;

    /// Strategy for a 32-byte digest wrapped as `ContentHash`.
    fn content_hash_strategy() -> impl Strategy<Value = ContentHash> {
        proptest::array::uniform32(any::<u8>()).prop_map(ContentHash::from_bytes)
    }

    /// Strategy for an arbitrary `SignalKey`. The grammar is
    /// `^[a-z][a-z0-9-]{0,126}$` — lead char MUST be `[a-z]`.
    fn signal_key_strategy() -> impl Strategy<Value = SignalKey> {
        "[a-z][a-z0-9-]{0,31}"
            .prop_map(|raw| SignalKey::new(&raw).expect("kebab body is a valid SignalKey"))
    }

    /// Strategy for an arbitrary [`WorkflowResult`] — the real terminal
    /// payload the journal `Terminal` command now carries (not a lossy
    /// label). Exercises all three variants, with `Failed` carrying an
    /// arbitrary `reason`, so the CBOR roundtrip proptest proves the full
    /// payload (including the reason) survives serialize→deserialize.
    fn workflow_result_strategy() -> impl Strategy<Value = WorkflowResult> {
        prop_oneof![
            Just(WorkflowResult::Success),
            "[a-zA-Z0-9 ]{0,32}".prop_map(|reason| WorkflowResult::Failed { reason }),
            Just(WorkflowResult::Cancelled),
        ]
    }

    /// Strategy for one arbitrary [`LoadedEntry`] — emits BOTH
    /// `Command(...)` (every replayable, cursor-advancing variant) and
    /// `Notification(...)` (the sole `SignalSeen` notification) so the
    /// roundtrip exercises the interleaved on-disk stream the store must
    /// preserve (D1/D2). NO `step` generator — identity is structural, the
    /// in-entry `step` was dropped (D5).
    fn loaded_entry_strategy() -> impl Strategy<Value = LoadedEntry> {
        let command = prop_oneof![
            (content_hash_strategy(), content_hash_strategy()).prop_map(
                |(spec_digest, input_digest)| JournalCommand::Started { spec_digest, input_digest }
            ),
            (
                "[a-z0-9-]{1,32}",
                content_hash_strategy(),
                proptest::collection::vec(any::<u8>(), 0..32)
            )
                .prop_map(|(name, result_digest, result_bytes)| {
                    JournalCommand::RunResult { name, result_digest, result_bytes }
                }),
            (0u64..u64::from(u32::MAX)).prop_map(|secs| JournalCommand::SleepArmed {
                deadline_unix: std::time::Duration::from_secs(secs),
            }),
            signal_key_strategy()
                .prop_map(|signal_key| JournalCommand::SignalAwaited { signal_key }),
            content_hash_strategy()
                .prop_map(|action_digest| JournalCommand::ActionEmitted { action_digest }),
            workflow_result_strategy().prop_map(|result| JournalCommand::Terminal { result }),
        ]
        .prop_map(LoadedEntry::Command);

        let notification = (signal_key_strategy(), content_hash_strategy(), "[a-zA-Z ]{0,32}")
            .prop_map(|(signal_key, value_digest, value)| {
                LoadedEntry::Notification(JournalNotification::SignalSeen {
                    signal_key,
                    value_digest,
                    value: SignalValue::new(value),
                })
            });

        prop_oneof![command, notification]
    }

    fn workflow_id() -> WorkflowId {
        WorkflowId::new("wf-test-0001").expect("valid id")
    }

    proptest! {
        /// Round-trip property (ADR-0063 §3): an arbitrary INTERLEAVED run
        /// of commands and notifications appended to a fresh instance
        /// loads back byte-equal and in append order. The
        /// `Symmetric`/`Roundtrip` Hebert-ch.3 pattern over the
        /// `Vec<LoadedEntry>` boundary representation (D1/D2) — the store
        /// is a dumb ordered log that preserves the interleave verbatim.
        #[test]
        fn append_then_load_round_trips_losslessly_and_in_order(
            entries in proptest::collection::vec(loaded_entry_strategy(), 0..16)
        ) {
            let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
            rt.block_on(async {
                let store = SimJournalStore::new();
                let id = workflow_id();
                for entry in &entries {
                    store.append(&id, entry).await.expect("append");
                }
                let loaded = store.load_journal(&id).await.expect("load");
                prop_assert_eq!(loaded, entries);
                Ok(())
            })?;
        }
    }

    #[tokio::test]
    async fn load_journal_for_unknown_instance_is_empty_not_error() {
        let store = SimJournalStore::new();
        let id = WorkflowId::new("wf-never-started").expect("valid id");
        let loaded = store.load_journal(&id).await.expect("load empty");
        assert!(loaded.is_empty(), "an instance with no entries loads as an empty run");
    }

    /// A `RunResult` command varied by `nonce` so successive entries in a
    /// run are distinct — a collision / overwrite is then observable as a
    /// missing or duplicated entry on load.
    fn run_result(nonce: &str) -> LoadedEntry {
        LoadedEntry::Command(JournalCommand::RunResult {
            name: "provision-write".to_string(),
            result_digest: ContentHash::of(nonce.as_bytes()),
            result_bytes: nonce.as_bytes().to_vec(),
        })
    }

    /// Regression guard mirroring the redb adapter's: the O(1) `next_step`
    /// reverse-peek must assign a fresh, non-colliding step on every append
    /// across a long contiguous run. A broken `next_step` returning a
    /// colliding step would make `append` overwrite a prior `BTreeMap`
    /// entry, so `load_journal` would return fewer distinct entries than
    /// were appended — observable through the public API. (The sim storage
    /// map is private, so unlike the redb test there is no raw-key
    /// assertion; the public-API distinctness check is the pin.)
    #[tokio::test]
    async fn next_step_assigns_contiguous_non_colliding_steps_across_a_long_run() {
        const K: u32 = 64;
        let store = SimJournalStore::new();
        let id = workflow_id();

        let expected: Vec<LoadedEntry> =
            (0..K).map(|n| run_result(&format!("entry-{n}"))).collect();
        for entry in &expected {
            store.append(&id, entry).await.expect("append");
        }

        let loaded = store.load_journal(&id).await.expect("load");
        assert_eq!(loaded.len(), K as usize, "every append must yield a distinct stored entry");
        assert_eq!(loaded, expected, "no collision, no overwrite, preserved append order");
    }

    #[tokio::test]
    async fn injected_fsync_failure_makes_append_fail_without_persisting() {
        let store = SimJournalStore::new();
        let id = workflow_id();
        let entry = LoadedEntry::Command(JournalCommand::Started {
            spec_digest: ContentHash::of(b"provision-record"),
            input_digest: ContentHash::of(b"provision-record"),
        });

        store.inject_fsync_failure();
        let err = store.append(&id, &entry).await.expect_err("injected append must fail");
        assert!(
            matches!(err, JournalStoreError::FsyncFailed { .. }),
            "injection surfaces as FsyncFailed, got {err:?}"
        );

        // Per ADR-0063 §4: the failed append left NO observable entry.
        let loaded = store.load_journal(&id).await.expect("load after failed append");
        assert!(loaded.is_empty(), "a failed append must not be observable in the journal");

        // After clearing, append succeeds and is observable.
        store.clear_fsync_failure();
        store.append(&id, &entry).await.expect("append after clear");
        let loaded = store.load_journal(&id).await.expect("load after clear");
        assert_eq!(loaded, vec![entry], "cleared failure restores normal append");
    }

    #[tokio::test]
    async fn probe_succeeds_clean_and_leaves_no_residue() {
        let store = SimJournalStore::new();
        store.probe().await.expect("clean probe succeeds");

        // The probe's sentinel must not leak into a real instance's run
        // nor remain under the probe id.
        let probe_id = WorkflowId::new(PROBE_WORKFLOW_ID).expect("valid probe id");
        let residue = store.load_journal(&probe_id).await.expect("load probe id");
        assert!(residue.is_empty(), "probe must delete its sentinel, leaving no residue");
    }

    #[tokio::test]
    async fn probe_under_injected_failure_refuses() {
        let store = SimJournalStore::new();
        store.inject_fsync_failure();
        let err = store.probe().await.expect_err("probe must fail under injection");
        assert!(
            matches!(err, ProbeError::WriteFailed { .. }),
            "injected probe failure surfaces as WriteFailed, got {err:?}"
        );
    }

    #[test]
    fn probe_sentinel_id_is_valid() {
        // The const must satisfy the WorkflowId validator — guards against
        // a future validator tightening silently breaking the probe.
        assert!(WorkflowId::new(PROBE_WORKFLOW_ID).is_ok());
    }
}
