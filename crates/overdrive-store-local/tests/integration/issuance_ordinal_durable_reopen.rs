//! Host-only durable-across-reopen sub-case for
//! `ObservationStore::next_issuance_ordinal` (architecture § 4.3 #3 /
//! ADR-0063 D6 rev 8 / fix-issuance-ordinal-toctou step 01-01).
//!
//! The sim adapter's durability domain is the sim-process lifetime (it
//! never reconstructs the store mid-run), so the durable-across-restart
//! invariant can only be exercised by the host adapter, which persists
//! the counter in a real redb file. This test allocates, drops and
//! reopens the `LocalObservationStore` on the SAME redb path, allocates
//! again, and asserts the post-reopen ordinal is strictly greater — the
//! committed counter survived process death.

use overdrive_core::traits::observation_store::ObservationStore;
use overdrive_store_local::LocalObservationStore;
use tempfile::TempDir;

#[tokio::test]
async fn issuance_ordinal_counter_survives_reopen() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("observation");

    // Allocate a handful of ordinals, then drop the store entirely —
    // simulating a process restart against the same on-disk redb file.
    let before_restart: u64 = {
        let store = LocalObservationStore::open(&path).expect("open observation store");
        let mut last = 0;
        for _ in 0..4 {
            last = store.next_issuance_ordinal().await.expect("allocate before restart").as_u64();
        }
        last
        // `store` dropped here — the redb file is closed.
    };

    // Reopen on the same path: the counter must NOT rewind.
    let store = LocalObservationStore::open(&path).expect("reopen observation store");
    let after_restart =
        store.next_issuance_ordinal().await.expect("allocate after restart").as_u64();

    assert!(
        after_restart > before_restart,
        "durable counter must survive reopen — the post-restart ordinal must be strictly \
         greater than the last pre-restart ordinal; before={before_restart}, \
         after={after_restart}"
    );
}
