//! Regression proptest pinning the commit-counter invariant:
//!
//! > `commit_index()` equals the count of effective committed
//! > operations that have advanced the cursor.
//!
//! The redb backend formerly bumped `commit_counter` BEFORE the redb
//! commit succeeded. If `table.insert` or `write.commit()` failed after
//! the bump, the counter advanced with no corresponding row on disk —
//! breaking the "`commit_index` reflects committed writes" invariant
//! documented in the module-header docstring of `redb_backend.rs`.
//!
//! The fix is bump-after-commit: peek the next index inside the
//! `spawn_blocking` body (redb serialises writers, so the load is
//! race-free), encode the per-entry frame with the peeked value,
//! commit, and only then `fetch_add(1)`. On commit failure the counter
//! is untouched.
//!
//! # What this test pins
//!
//! For an arbitrary sequence of `put` / `delete` / `txn` operations
//! against a fresh `LocalIntentStore`:
//!
//! 1. `commit_index() == effective_op_count` — every effective op
//!    advances the cursor by exactly 1, regardless of mix.
//! 2. The cursor never overtakes the count of effective committed ops.
//!    A bump-before-commit regression that fires on a no-op delete or
//!    a no-op transaction would surface as `commit_index >
//!    effective_op_count` here.
//!
//! "Effective op" semantics match the existing `phantom_writes.rs`
//! acceptance contract:
//!
//! * `put(k, v)` is always effective.
//! * `delete(k)` is effective iff the key was present.
//! * `txn(ops)` is effective iff at least one op inside has effect
//!   (txn-level bump is one regardless of op count).
//!
//! # Why no real fault-injection
//!
//! A genuine bump-then-redb-commit-fails reproduction would require
//! either filling the tempdir's disk quota or forcing a redb internal
//! error mid-commit — both brittle / non-portable on tempfile-backed
//! filesystems. The happy-path invariant above is the floor: it
//! catches every regression where the per-op bump count drifts (e.g.
//! a mutation that bumps twice on one path, or fails to bump on
//! another), and the mutation-testing gate (`cargo xtask mutants
//! --diff origin/main --package overdrive-store-local`) provides the
//! per-mutation kill-rate floor that would catch a "skip the bump on
//! commit failure" regression even without I/O fault injection.
//!
//! # Test lane
//!
//! Lives under `tests/integration/` because every case opens a real
//! redb file in a `TempDir` and runs O(N) ops per case at the default
//! 256-case proptest budget. The `tests/integration.rs` entrypoint
//! gates the whole binary behind the `integration-tests` feature per
//! `.claude/rules/testing.md` "Integration vs unit gating".

use bytes::Bytes;
use overdrive_core::traits::intent_store::{IntentStore, TxnOp, TxnOutcome};
use overdrive_store_local::LocalIntentStore;
use proptest::prelude::*;
use tempfile::TempDir;
use tokio::runtime::Runtime;

/// One step in a generated sequence.
#[derive(Clone, Debug)]
enum Step {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
    Txn { ops: Vec<TxnStep> },
}

/// One op inside a generated transaction.
#[derive(Clone, Debug)]
enum TxnStep {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

/// Generator for a single `Step`.
///
/// Keys are drawn from a small alphabet so deletes have a non-trivial
/// chance of hitting a previously-written key — otherwise every
/// `delete` would land on an absent key, which is also a phantom-write
/// path but loses the "effective delete advances counter" coverage.
fn step_strategy() -> impl Strategy<Value = Step> {
    let key = prop::collection::vec(0u8..16, 1..=4);
    let value = prop::collection::vec(any::<u8>(), 0..=32);

    let put = (key.clone(), value.clone()).prop_map(|(k, v)| Step::Put { key: k, value: v });
    let delete = key.clone().prop_map(|k| Step::Delete { key: k });
    let txn = prop::collection::vec(
        prop_oneof![
            (key.clone(), value).prop_map(|(k, v)| TxnStep::Put { key: k, value: v }),
            key.prop_map(|k| TxnStep::Delete { key: k }),
        ],
        0..=4,
    )
    .prop_map(|ops| Step::Txn { ops });

    prop_oneof![put, delete, txn]
}

/// Driver model: an in-test mirror of the store's effective-op count.
///
/// Every `apply` call mirrors what `LocalIntentStore` should do, and
/// returns whether the op was *effective* (would advance the counter).
/// The aggregate of returned effective counts is the expected value of
/// `commit_index()` after the full sequence.
#[derive(Default)]
struct Model {
    state: std::collections::BTreeMap<Vec<u8>, Vec<u8>>,
    effective_ops: u64,
}

impl Model {
    fn apply_put(&mut self, key: &[u8], value: &[u8]) {
        // Put always has effect — overwriting an existing value is
        // still a committed write that advances the cursor.
        self.state.insert(key.to_vec(), value.to_vec());
        self.effective_ops += 1;
    }

    fn apply_delete(&mut self, key: &[u8]) {
        // Delete is effective iff the key was present.
        if self.state.remove(key).is_some() {
            self.effective_ops += 1;
        }
    }

    fn apply_txn(&mut self, ops: &[TxnStep]) {
        if ops.is_empty() {
            // Empty txn short-circuits — no commit, no bump.
            return;
        }
        let mut any_effective = false;
        for op in ops {
            match op {
                TxnStep::Put { key, value } => {
                    self.state.insert(key.clone(), value.clone());
                    any_effective = true;
                }
                TxnStep::Delete { key } => {
                    if self.state.remove(key).is_some() {
                        any_effective = true;
                    }
                }
            }
        }
        if any_effective {
            // Txn-level bump: one regardless of op count.
            self.effective_ops += 1;
        }
    }
}

proptest! {
    /// Property — for any sequence of put / delete / txn against a
    /// fresh `LocalIntentStore`, `commit_index()` equals the count of
    /// effective committed operations the store has accepted.
    ///
    /// A regression where the counter bumps on a no-op (e.g. delete of
    /// an absent key, empty txn) or fails to bump on an effective op
    /// surfaces as a final-state inequality between the store and the
    /// model. The model is the canonical specification of which ops
    /// are effective.
    #[test]
    fn commit_index_equals_effective_committed_op_count_for_any_sequence(
        steps in prop::collection::vec(step_strategy(), 0..=64)
    ) {
        let rt = Runtime::new().expect("runtime");
        rt.block_on(async move {
            let tmp = TempDir::new().expect("temp dir");
            let store = LocalIntentStore::open(tmp.path().join("intent.redb"))
                .expect("open store");

            let mut model = Model::default();

            for step in &steps {
                match step {
                    Step::Put { key, value } => {
                        store.put(key, value).await.expect("put commits");
                        model.apply_put(key, value);
                    }
                    Step::Delete { key } => {
                        store.delete(key).await.expect("delete commits");
                        model.apply_delete(key);
                    }
                    Step::Txn { ops } => {
                        let txn_ops: Vec<TxnOp> = ops
                            .iter()
                            .map(|op| match op {
                                TxnStep::Put { key, value } => TxnOp::Put {
                                    key: Bytes::copy_from_slice(key),
                                    value: Bytes::copy_from_slice(value),
                                },
                                TxnStep::Delete { key } => TxnOp::Delete {
                                    key: Bytes::copy_from_slice(key),
                                },
                            })
                            .collect();
                        let outcome = store.txn(txn_ops).await.expect("txn commits");
                        prop_assert!(
                            matches!(outcome, TxnOutcome::Committed),
                            "txn must commit on happy path, got {outcome:?}",
                        );
                        model.apply_txn(ops);
                    }
                }

                // Per-step invariant: the store's counter must match
                // the model exactly. A bump-before-commit regression
                // that advanced the counter on a no-op delete or empty
                // txn would surface immediately at the boundary, with
                // a minimised counter-example via proptest shrinking.
                prop_assert_eq!(
                    store.commit_index(),
                    model.effective_ops,
                    "commit_index must equal effective committed op count; \
                     a phantom bump on a no-op or a missing bump on an \
                     effective op breaks this invariant",
                );
            }

            // Final-state assertion (redundant with the loop, retained
            // as a separate landmark in the test report).
            prop_assert_eq!(store.commit_index(), model.effective_ops);
            Ok(())
        })?;
    }
}
