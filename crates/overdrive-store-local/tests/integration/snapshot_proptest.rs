//! Proptest — mandatory snapshot-roundtrip call site.
//!
//! Per `.claude/rules/testing.md` *Mandatory call sites*:
//!
//! > **Snapshot roundtrip.** `IntentStore::export_snapshot` →
//! > `bootstrap_from` → `export_snapshot` is bit-identical. The
//! > non-destructive single → HA migration story depends on this.
//!
//! Translates `docs/feature/phase-1-foundation/distill/test-scenarios.md`
//! §4.2 property scenario — "Snapshot round-trip is byte-identical for
//! any valid store contents" — to a proptest. The generator produces
//! store contents with 0..=256 entries, each key 1..=64 bytes and each
//! value 0..=4096 bytes, with unique keys within a generation. This
//! matches the roadmap scope for step 03-02.
//!
//! Port-to-port discipline: the property drives `IntentStore::put`,
//! `IntentStore::export_snapshot`, and `IntentStore::bootstrap_from`
//! exclusively. Internal rkyv details are not inspected; only the
//! framed byte slice returned by `StateSnapshot::bytes()` is compared.
//!
//! Lives under `tests/integration/` because every case opens a real
//! redb file in a `TempDir` — touching the filesystem at the default
//! 1024-case budget routinely crosses the unit-lane wall-clock budget.
//! The entrypoint at `tests/integration.rs` enforces the
//! `integration-tests` feature gate; this module inherits it.

use std::collections::BTreeMap;

use overdrive_core::traits::intent_store::IntentStore;
use overdrive_store_local::LocalIntentStore;
use proptest::prelude::*;
use tempfile::TempDir;
use tokio::runtime::Runtime;

/// Generator for valid store contents.
///
/// * 0..=256 entries (covers empty through a meaningful upper bound).
/// * Each key is 1..=64 bytes (non-empty; 64 is a sane headroom for any
///   identifier Phase 1 writes into `entries`).
/// * Each value is 0..=4096 bytes (4 KB covers the §4.2 scope; empty
///   values are permitted because `put(key, &[])` is legal on the
///   trait).
/// * Keys within a single generation are unique — a `BTreeMap` collapses
///   duplicates before we return. Ordering does not leak: the store's
///   export re-sorts anyway.
fn store_contents() -> impl Strategy<Value = Vec<(Vec<u8>, Vec<u8>)>> {
    prop::collection::vec(
        (prop::collection::vec(any::<u8>(), 1..=64), prop::collection::vec(any::<u8>(), 0..=4096)),
        0..=256,
    )
    .prop_map(|pairs| {
        // Deduplicate keys — last write for a given key wins, matching
        // how the store itself would resolve duplicate puts.
        let map: BTreeMap<Vec<u8>, Vec<u8>> = pairs.into_iter().collect();
        map.into_iter().collect()
    })
}

proptest! {
    /// Property — for any valid store contents, exporting a snapshot,
    /// bootstrapping a fresh store from it, and re-exporting produces
    /// a byte-identical export.
    ///
    /// This is the testing.md mandatory call site that proves
    /// single → HA migration correctness holds under fuzzed inputs,
    /// not just the hand-picked entries in the acceptance tests.
    #[test]
    fn snapshot_roundtrip_is_byte_identical_for_arbitrary_store_contents(
        contents in store_contents()
    ) {
        // Build a fresh runtime per case — proptest runs inside a
        // synchronous `#[test]`, so we cannot use `#[tokio::test]`. The
        // cost is acceptable at the default 1024-case budget; the
        // runtime is cheap compared to the redb I/O this already
        // performs.
        let rt = Runtime::new().expect("runtime");
        rt.block_on(async move {
            // Source store: insert every (key, value) pair exactly
            // once. The generator guarantees unique keys.
            let tmp_src = TempDir::new().expect("temp dir src");
            let src = LocalIntentStore::open(tmp_src.path().join("intent.redb"))
                .expect("open src");
            for (key, value) in &contents {
                src.put(key, value).await.expect("put src");
            }

            let snap_src = src.export_snapshot().await.expect("export src");

            // Target store: bootstrap from the source snapshot.
            let tmp_dst = TempDir::new().expect("temp dir dst");
            let dst = LocalIntentStore::open(tmp_dst.path().join("intent.redb"))
                .expect("open dst");
            dst.bootstrap_from(snap_src.clone())
                .await
                .expect("bootstrap dst");

            let snap_dst = dst.export_snapshot().await.expect("export dst");

            prop_assert_eq!(
                snap_src.bytes(),
                snap_dst.bytes(),
                "snapshot bytes must be byte-identical across round-trip"
            );
            Ok(())
        })?;
    }
}
